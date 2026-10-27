import { useState, useMemo, useCallback, useRef, useEffect } from 'react'
import {
  useMqttBrokerStatus,
  useStartMqttBroker,
  useStopMqttBroker,
  useUpdateMqttConfig,
  useMqttTopics,
  useMqttMessages,
  useMqttTopicMessages,
  useMqttPublish,
} from './queries'
import type { TopicTree, TopicInfo, RecentMessage } from './queries'
import { inputStyle, btnStyle, tabStyle } from './styles'

type Tab = 'overview' | 'topics' | 'messages' | 'publish'

// ── Topic Tree Node ──────────────────────────────────────────────

interface TreeNodeProps {
  name: string
  data: TopicTree
  path: string
  selected: string | null
  onSelect: (topic: string) => void
  topicsMap: Map<string, TopicInfo>
}

function TopicTreeNode({ name, data, path, selected, onSelect, topicsMap }: TreeNodeProps) {
  const [expanded, setExpanded] = useState(false)
  const fullTopic = path ? `${path}/${name}` : name
  const keys = Object.keys(data)
  const children = keys.filter(k => !k.startsWith('_'))
  const leaf = (data as any)._leaf === true
  const msgCount = (data as any)._message_count ?? 0
  const info = topicsMap.get(fullTopic)
  const isSelected = selected === fullTopic

  return (
    <div style={{ marginLeft: path ? 16 : 0 }}>
      <div
        onClick={() => {
          if (leaf) onSelect(fullTopic)
        }}
        style={{
          display: 'flex',
          alignItems: 'center',
          gap: 6,
          padding: '5px 8px',
          borderRadius: 6,
          cursor: leaf ? 'pointer' : 'default',
          background: isSelected ? 'rgba(34,197,94,.1)' : 'transparent',
          border: isSelected ? '1px solid var(--cyan)' : '1px solid transparent',
          transition: 'background .15s',
        }}
      >
        {children.length > 0 ? (
          <button
            onClick={e => { e.stopPropagation(); setExpanded(v => !v) }}
            style={{ background: 'none', border: 'none', color: 'var(--muted)', cursor: 'pointer', fontSize: 10, padding: 2 }}
          >
            {expanded ? '▼' : '▶'}
          </button>
        ) : (
          <span style={{ width: 14, display: 'inline-flex', alignItems: 'center', justifyContent: 'center', fontSize: 10, color: 'var(--muted)' }}>●</span>
        )}
        <span style={{ fontSize: 13, fontWeight: 500, color: leaf ? 'var(--fg)' : 'var(--muted)' }}>{name}</span>
        {msgCount > 0 && (
          <span style={{
            fontSize: 10,
            color: 'var(--cyan)',
            background: 'rgba(34,211,238,.1)',
            padding: '1px 6px',
            borderRadius: 8,
            marginLeft: 'auto',
            fontFamily: 'var(--font-mono)',
          }}>
            {msgCount.toLocaleString()}
          </span>
        )}
      </div>
      {expanded && children.length > 0 && (
        <div>
          {children.map(child => (
            <TopicTreeNode
              key={child}
              name={child}
              data={(data as any)[child]}
              path={fullTopic}
              selected={selected}
              onSelect={onSelect}
              topicsMap={topicsMap}
            />
          ))}
        </div>
      )}
    </div>
  )
}

// ── Payload viewer ───────────────────────────────────────────────

function PayloadView({ payload }: { payload: string }) {
  let formatted = payload
  let isJson = false
  try {
    const parsed = JSON.parse(payload)
    formatted = JSON.stringify(parsed, null, 2)
    isJson = true
  } catch {}

  return (
    <pre style={{
      margin: 0,
      fontSize: 12,
      fontFamily: 'var(--font-mono)',
      color: isJson ? 'var(--cyan)' : 'var(--fg)',
      whiteSpace: 'pre-wrap',
      wordBreak: 'break-all',
      lineHeight: 1.5,
    }}>
      {formatted}
    </pre>
  )
}

// ── Helpers ──────────────────────────────────────────────────────

function formatTimestamp(ts: number) {
  return new Date(ts * 1000).toLocaleTimeString('en-US', { hour12: false })
}

function timeAgo(ts: number) {
  const diff = Math.max(0, Math.floor(Date.now() / 1000) - ts)
  if (diff < 60) return `${diff}s ago`
  if (diff < 3600) return `${Math.floor(diff / 60)}m ago`
  return `${Math.floor(diff / 3600)}h ago`
}

// ── Overview Tab ─────────────────────────────────────────────────

function OverviewTab() {
  const { data: status, isLoading } = useMqttBrokerStatus()
  const startMut = useStartMqttBroker()
  const stopMut = useStopMqttBroker()
  const isRunning = status?.status === 'running'

  if (isLoading) {
    return <div style={{ padding: 40, textAlign: 'center', color: 'var(--muted)' }}>Loading broker status...</div>
  }

  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 16 }}>
      {/* Status card */}
      <div style={{ ...cardStyle, display: 'flex', alignItems: 'center', justifyContent: 'space-between' }}>
        <div style={{ display: 'flex', alignItems: 'center', gap: 12 }}>
          <div style={{
            width: 14, height: 14, borderRadius: '50%',
            background: isRunning ? 'var(--green)' : 'var(--crit)',
            boxShadow: `0 0 10px ${isRunning ? 'var(--green)' : 'var(--crit)'}`,
          }} />
          <div>
            <div style={{ fontSize: 16, fontWeight: 700 }}>MQTT Broker</div>
            <div style={{ fontSize: 12, color: 'var(--muted)' }}>
              {isRunning ? `Running on port ${status?.config?.port ?? 1883}` : 'Stopped'}
            </div>
          </div>
        </div>
        <button
          onClick={() => isRunning ? stopMut.mutate() : startMut.mutate()}
          disabled={startMut.isPending || stopMut.isPending}
          style={{
            ...btnStyle,
            borderColor: isRunning ? 'var(--crit)' : 'var(--green)',
            color: isRunning ? 'var(--crit)' : 'var(--green)',
          }}
        >
          {startMut.isPending || stopMut.isPending
            ? (isRunning ? 'Stopping...' : 'Starting...')
            : (isRunning ? 'Stop' : 'Start')}
        </button>
      </div>

      {/* Connected Clients */}
      {isRunning && (
        <div style={cardStyle}>
          <div style={{ fontSize: 12, fontWeight: 600, marginBottom: 12, color: 'var(--muted)', textTransform: 'uppercase', letterSpacing: '.05em' }}>
            Connected Clients ({status?.clients?.length ?? 0})
          </div>
          {(!status?.clients || status.clients.length === 0) ? (
            <div style={{ padding: 24, textAlign: 'center', color: 'var(--muted)', fontSize: 13, background: 'var(--bg)', borderRadius: 8 }}>
              No clients connected
            </div>
          ) : (
            <div style={{ overflow: 'auto', background: 'var(--bg)', borderRadius: 8, border: '1px solid var(--border)' }}>
              <table style={{ width: '100%', borderCollapse: 'collapse' }}>
                <thead>
                  <tr>
                    <th style={thStyle}>Client ID</th>
                    <th style={thStyle}>Connected</th>
                    <th style={thStyle}>Subscriptions</th>
                  </tr>
                </thead>
                <tbody>
                  {status.clients.map(c => (
                    <tr key={c.client_id}>
                      <td style={{ ...tdStyle, fontFamily: 'var(--font-mono)', fontSize: 12 }}>{c.client_id}</td>
                      <td style={tdStyle}>{timeAgo(c.connected_at)}</td>
                      <td style={tdStyle}>
                        <div style={{ display: 'flex', flexWrap: 'wrap', gap: 4 }}>
                          {c.subscriptions.length > 0
                            ? c.subscriptions.map(s => (
                              <span key={s} style={{ fontSize: 10, padding: '2px 6px', borderRadius: 4, background: 'rgba(34,211,238,.1)', color: 'var(--cyan)', fontFamily: 'var(--font-mono)' }}>{s}</span>
                            ))
                            : <span style={{ color: 'var(--muted)' }}>-</span>}
                        </div>
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          )}
        </div>
      )}
    </div>
  )
}

// ── Topics Tab ───────────────────────────────────────────────────

function TopicsTab() {
  const { data: topicsData, isLoading } = useMqttTopics()
  const [selectedTopic, setSelectedTopic] = useState<string | null>(null)
  const { data: topicMsgs } = useMqttTopicMessages(selectedTopic)
  const [search, setSearch] = useState('')

  const topicsMap = useMemo(() => {
    const map = new Map<string, TopicInfo>()
    if (topicsData?.topics) {
      for (const t of topicsData.topics) map.set(t.topic, t)
    }
    return map
  }, [topicsData?.topics])

  const filteredTopics = useMemo(() => {
    if (!topicsData?.topics) return []
    if (!search) return topicsData.topics
    const lower = search.toLowerCase()
    return topicsData.topics.filter(t => t.topic.toLowerCase().includes(lower))
  }, [topicsData?.topics, search])

  if (isLoading) {
    return <div style={{ padding: 40, textAlign: 'center', color: 'var(--muted)' }}>Loading topics...</div>
  }

  return (
    <div style={{ display: 'flex', gap: 16, height: '100%' }}>
      {/* Tree panel */}
      <div style={{ flex: 1, display: 'flex', flexDirection: 'column', gap: 8, minWidth: 0 }}>
        <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between' }}>
          <span style={{ fontSize: 12, fontWeight: 600, color: 'var(--muted)', textTransform: 'uppercase' }}>
            Topic Tree ({topicsData?.topics?.length ?? 0})
          </span>
        </div>
        <input
          placeholder="Filter topics..."
          value={search}
          onChange={e => setSearch(e.target.value)}
          style={inputStyle}
        />
        <div style={{ flex: 1, overflow: 'auto', background: 'var(--bg)', borderRadius: 8, border: '1px solid var(--border)', padding: 8 }}>
          {search ? (
            // Flat filtered list
            filteredTopics.length === 0 ? (
              <div style={{ padding: 16, textAlign: 'center', color: 'var(--muted)', fontSize: 12 }}>No matching topics</div>
            ) : (
              filteredTopics.map(t => (
                <div
                  key={t.topic}
                  onClick={() => setSelectedTopic(t.topic)}
                  style={{
                    padding: '4px 8px',
                    borderRadius: 4,
                    cursor: 'pointer',
                    background: selectedTopic === t.topic ? 'rgba(34,197,94,.1)' : 'transparent',
                    borderLeft: selectedTopic === t.topic ? '2px solid var(--cyan)' : '2px solid transparent',
                    marginBottom: 2,
                  }}
                >
                  <div style={{ fontSize: 12, fontFamily: 'var(--font-mono)', color: 'var(--fg)' }}>{t.topic}</div>
                  <div style={{ display: 'flex', gap: 8, marginTop: 2 }}>
                    <span style={{ fontSize: 10, color: 'var(--muted)' }}>{timeAgo(t.last_update)}</span>
                    <span style={{ fontSize: 10, color: 'var(--cyan)' }}>{t.message_count.toLocaleString()} msgs</span>
                  </div>
                </div>
              ))
            )
          ) : (
            // Tree view
            topicsData?.tree && Object.keys(topicsData.tree).length > 0 ? (
              Object.entries(topicsData.tree).map(([key, val]) => (
                <TopicTreeNode
                  key={key}
                  name={key}
                  data={val as TopicTree}
                  path=""
                  selected={selectedTopic}
                  onSelect={setSelectedTopic}
                  topicsMap={topicsMap}
                />
              ))
            ) : (
              <div style={{ padding: 16, textAlign: 'center', color: 'var(--muted)', fontSize: 12 }}>
                No topics yet. Connect a client and publish messages to see topics appear here.
              </div>
            )
          )}
        </div>
      </div>

      {/* Detail panel */}
      <div style={{ width: 340, flexShrink: 0, display: 'flex', flexDirection: 'column', gap: 8 }}>
        {selectedTopic ? (
          <>
            <div style={{ ...cardStyle, flexShrink: 0 }}>
              <div style={{ fontSize: 11, color: 'var(--muted)', textTransform: 'uppercase', marginBottom: 6 }}>Selected Topic</div>
              <div style={{ fontSize: 13, fontFamily: 'var(--font-mono)', fontWeight: 600, wordBreak: 'break-all' }}>{selectedTopic}</div>
              {topicsMap.has(selectedTopic) && (
                <div style={{ display: 'flex', gap: 12, marginTop: 8 }}>
                  <span style={{ fontSize: 11, color: 'var(--muted)' }}>Messages: <span style={{ color: 'var(--cyan)' }}>{topicsMap.get(selectedTopic)!.message_count.toLocaleString()}</span></span>
                  <span style={{ fontSize: 11, color: 'var(--muted)' }}>Updated: <span style={{ color: 'var(--fg)' }}>{timeAgo(topicsMap.get(selectedTopic)!.last_update)}</span></span>
                </div>
              )}
            </div>
            {/* Last value */}
            {topicsMap.has(selectedTopic) && (
              <div style={cardStyle}>
                <div style={{ fontSize: 11, color: 'var(--muted)', textTransform: 'uppercase', marginBottom: 6 }}>Last Value</div>
                <div style={{ background: 'var(--bg)', borderRadius: 6, padding: 10, maxHeight: 120, overflow: 'auto' }}>
                  <PayloadView payload={topicsMap.get(selectedTopic)!.last_payload} />
                </div>
              </div>
            )}
            {/* Recent messages */}
            <div style={{ ...cardStyle, flex: 1, overflow: 'hidden', display: 'flex', flexDirection: 'column' }}>
              <div style={{ fontSize: 11, color: 'var(--muted)', textTransform: 'uppercase', marginBottom: 6 }}>Recent Messages</div>
              <div style={{ flex: 1, overflow: 'auto' }}>
                {topicMsgs?.messages && topicMsgs.messages.length > 0 ? (
                  topicMsgs.messages.map((m, i) => (
                    <div key={i} style={{ padding: '6px 0', borderBottom: '1px solid var(--border)' }}>
                      <div style={{ display: 'flex', justifyContent: 'space-between', marginBottom: 2 }}>
                        <span style={{ fontSize: 10, color: 'var(--muted)' }}>{formatTimestamp(m.timestamp)}</span>
                        {m.retain && <span style={{ fontSize: 9, padding: '1px 4px', borderRadius: 3, background: 'rgba(250,204,21,.15)', color: '#facc15' }}>RETAIN</span>}
                      </div>
                      <div style={{ background: 'var(--bg)', borderRadius: 4, padding: '4px 6px', maxHeight: 60, overflow: 'auto' }}>
                        <PayloadView payload={m.payload} />
                      </div>
                    </div>
                  ))
                ) : (
                  <div style={{ fontSize: 12, color: 'var(--muted)', textAlign: 'center', padding: 16 }}>No messages yet</div>
                )}
              </div>
            </div>
          </>
        ) : (
          <div style={{ ...cardStyle, display: 'flex', alignItems: 'center', justifyContent: 'center', height: '100%' }}>
            <div style={{ textAlign: 'center', color: 'var(--muted)' }}>
              <div style={{ fontSize: 13, fontWeight: 600, color: 'var(--cyan)', marginBottom: 8 }}>MQTT</div>
              <div style={{ fontSize: 13 }}>Select a topic to view details</div>
            </div>
          </div>
        )}
      </div>
    </div>
  )
}

// ── Messages Tab ─────────────────────────────────────────────────

function MessagesTab() {
  const { data, isLoading } = useMqttMessages()
  const [filter, setFilter] = useState('')
  const bottomRef = useRef<HTMLDivElement>(null)
  const [autoScroll, setAutoScroll] = useState(true)
  const containerRef = useRef<HTMLDivElement>(null)

  const messages = useMemo(() => {
    if (!data?.messages) return []
    if (!filter) return data.messages
    const lower = filter.toLowerCase()
    return data.messages.filter(m => m.topic.toLowerCase().includes(lower))
  }, [data?.messages, filter])

  useEffect(() => {
    if (autoScroll && bottomRef.current) {
      bottomRef.current.scrollIntoView({ behavior: 'smooth' })
    }
  }, [messages.length, autoScroll])

  const handleScroll = useCallback(() => {
    const el = containerRef.current
    if (!el) return
    const atBottom = el.scrollHeight - el.scrollTop - el.clientHeight < 50
    setAutoScroll(atBottom)
  }, [])

  if (isLoading) {
    return <div style={{ padding: 40, textAlign: 'center', color: 'var(--muted)' }}>Loading messages...</div>
  }

  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 8, height: '100%' }}>
      <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between' }}>
        <span style={{ fontSize: 12, fontWeight: 600, color: 'var(--muted)', textTransform: 'uppercase' }}>
          Message Stream ({messages.length})
        </span>
        {autoScroll && <span style={{ fontSize: 10, color: 'var(--green)' }}>Live</span>}
      </div>
      <input
        placeholder="Filter by topic..."
        value={filter}
        onChange={e => setFilter(e.target.value)}
        style={inputStyle}
      />
      <div
        ref={containerRef}
        onScroll={handleScroll}
        style={{ flex: 1, overflow: 'auto', background: 'var(--bg)', borderRadius: 8, border: '1px solid var(--border)', padding: 4 }}
      >
        {messages.length === 0 ? (
          <div style={{ padding: 40, textAlign: 'center', color: 'var(--muted)', fontSize: 13 }}>
            No messages yet. Publish a message to see it here.
          </div>
        ) : (
          messages.map((m, i) => (
            <div
              key={i}
              style={{
                display: 'grid',
                gridTemplateColumns: '80px 1fr auto',
                gap: 12,
                alignItems: 'start',
                padding: '8px 10px',
                borderBottom: '1px solid var(--border)',
                fontSize: 12,
              }}
            >
              <span style={{ color: 'var(--muted)', fontFamily: 'var(--font-mono)', fontSize: 11 }}>{formatTimestamp(m.timestamp)}</span>
              <div style={{ minWidth: 0 }}>
                <div style={{ fontFamily: 'var(--font-mono)', fontSize: 11, color: 'var(--cyan)', marginBottom: 2, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>{m.topic}</div>
                <div style={{ maxHeight: 60, overflow: 'auto' }}>
                  <PayloadView payload={m.payload} />
                </div>
              </div>
              <div style={{ display: 'flex', flexDirection: 'column', gap: 2, alignItems: 'flex-end' }}>
                {m.retain && <span style={{ fontSize: 9, padding: '1px 4px', borderRadius: 3, background: 'rgba(250,204,21,.15)', color: '#facc15' }}>RETAIN</span>}
              </div>
            </div>
          ))
        )}
        <div ref={bottomRef} />
      </div>
    </div>
  )
}

// ── Publish Tab ──────────────────────────────────────────────────

function PublishTab() {
  const publishMut = useMqttPublish()
  const [topic, setTopic] = useState('')
  const [payload, setPayload] = useState('')
  const { data: topicsData } = useMqttTopics()

  const suggestions = useMemo(() => {
    if (!topicsData?.topics || !topic) return []
    const lower = topic.toLowerCase()
    return topicsData.topics
      .filter(t => t.topic.toLowerCase().startsWith(lower))
      .slice(0, 8)
      .map(t => t.topic)
  }, [topicsData?.topics, topic])

  const handlePublish = (e: React.FormEvent) => {
    e.preventDefault()
    if (!topic.trim()) return
    publishMut.mutate(
      { topic: topic.trim(), payload },
      { onSuccess: () => { setPayload('') } },
    )
  }

  return (
    <div style={{ maxWidth: 600 }}>
      <form onSubmit={handlePublish} style={{ ...cardStyle, display: 'flex', flexDirection: 'column', gap: 16 }}>
        <div style={{ fontSize: 14, fontWeight: 600 }}>Publish Message</div>

        <div style={{ display: 'flex', flexDirection: 'column', gap: 4, position: 'relative' }}>
          <label style={{ fontSize: 11, color: 'var(--muted)', textTransform: 'uppercase' }}>Topic</label>
          <input
            value={topic}
            onChange={e => setTopic(e.target.value)}
            placeholder="e.g. devices/sensor1/temperature"
            style={inputStyle}
            required
          />
          {suggestions.length > 0 && (
            <div style={{
              position: 'absolute', top: '100%', left: 0, right: 0,
              background: 'var(--card)', border: '1px solid var(--border)',
              borderRadius: 6, zIndex: 10, maxHeight: 200, overflow: 'auto',
            }}>
              {suggestions.map(s => (
                <div
                  key={s}
                  onClick={() => setTopic(s)}
                  style={{ padding: '6px 10px', fontSize: 12, cursor: 'pointer', fontFamily: 'var(--font-mono)' }}
                  onMouseEnter={e => (e.currentTarget.style.background = 'var(--bg)')}
                  onMouseLeave={e => (e.currentTarget.style.background = 'transparent')}
                >
                  {s}
                </div>
              ))}
            </div>
          )}
        </div>

        <div style={{ display: 'flex', flexDirection: 'column', gap: 4 }}>
          <label style={{ fontSize: 11, color: 'var(--muted)', textTransform: 'uppercase' }}>Payload</label>
          <textarea
            value={payload}
            onChange={e => setPayload(e.target.value)}
            placeholder='{"temperature": 25.5}'
            style={{
              ...inputStyle,
              minHeight: 120,
              resize: 'vertical',
              fontFamily: 'var(--font-mono)',
              fontSize: 12,
              lineHeight: 1.6,
            }}
          />
        </div>

        {publishMut.error && (
          <div style={{ color: 'var(--crit)', fontSize: 12 }}>
            {(publishMut.error as Error).message}
          </div>
        )}

        <div style={{ display: 'flex', gap: 8, justifyContent: 'flex-end' }}>
          <button
            type="submit"
            disabled={publishMut.isPending || !topic.trim()}
            style={{
              ...btnStyle,
              borderColor: 'var(--green)',
              color: 'var(--green)',
              opacity: !topic.trim() ? 0.5 : 1,
            }}
          >
            {publishMut.isPending ? 'Publishing...' : 'Publish'}
          </button>
        </div>
      </form>
    </div>
  )
}

// ── Config Panel ─────────────────────────────────────────────────

function ConfigPanel({ show, onClose }: { show: boolean; onClose: () => void }) {
  const { data: status } = useMqttBrokerStatus()
  const updateMut = useUpdateMqttConfig()
  const [port, setPort] = useState<number>(1883)
  const [maxConnections, setMaxConnections] = useState<number>(100)
  const [authEnabled, setAuthEnabled] = useState(false)
  const [users, setUsers] = useState<{ username: string; password: string }[]>([])

  useEffect(() => {
    if (status?.config) {
      setPort(status.config.port)
      setMaxConnections(status.config.max_connections)
      setAuthEnabled(status.config.auth_enabled)
    }
  }, [status?.config])

  if (!show) return null

  const handleSave = (e: React.FormEvent) => {
    e.preventDefault()
    // The server keeps users not in the list, so send only users typed here.
    const named = users.filter(u => u.username.trim() !== '')
    updateMut.mutate({
      port,
      max_connections: maxConnections,
      auth: {
        enabled: authEnabled,
        users: authEnabled && named.length > 0 ? named : undefined,
      },
    }, { onSuccess: onClose })
  }

  return (
    <form onSubmit={handleSave} style={{ ...cardStyle, maxWidth: 500, display: 'flex', flexDirection: 'column', gap: 16 }}>
      <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 16 }}>
        <label style={{ display: 'flex', flexDirection: 'column', gap: 4 }}>
          <span style={{ fontSize: 11, color: 'var(--muted)', textTransform: 'uppercase' }}>Port</span>
          <input type="number" value={port} onChange={e => setPort(Number(e.target.value))} style={inputStyle} min={1} max={65535} />
        </label>
        <label style={{ display: 'flex', flexDirection: 'column', gap: 4 }}>
          <span style={{ fontSize: 11, color: 'var(--muted)', textTransform: 'uppercase' }}>Max Connections</span>
          <input type="number" value={maxConnections} onChange={e => setMaxConnections(Number(e.target.value))} style={inputStyle} min={1} />
        </label>
      </div>

      <div style={{ display: 'flex', alignItems: 'center', gap: 12, padding: 12, background: 'var(--bg)', borderRadius: 8 }}>
        <span style={{ fontSize: 13, fontWeight: 500 }}>Authentication Required</span>
        <button
          type="button"
          onClick={() => setAuthEnabled(!authEnabled)}
          style={{
            width: 40, height: 22, borderRadius: 11, border: 'none', cursor: 'pointer',
            background: authEnabled ? 'var(--green)' : 'var(--border)',
            position: 'relative', transition: 'background 0.2s',
          }}
        >
          <span style={{
            position: 'absolute', top: 2,
            left: authEnabled ? 20 : 2,
            width: 18, height: 18, borderRadius: '50%', background: '#fff',
            transition: 'left 0.2s',
          }} />
        </button>
      </div>

      {authEnabled && (
        <div>
          <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', marginBottom: 8 }}>
            <span style={{ fontSize: 12, fontWeight: 600, color: 'var(--muted)', textTransform: 'uppercase' }}>Users</span>
            <button type="button" style={{ ...btnStyle, fontSize: 11, padding: '4px 10px', borderColor: 'var(--cyan)', color: 'var(--cyan)' }} onClick={() => setUsers([...users, { username: '', password: '' }])}>
              + Add User
            </button>
          </div>
          {users.length === 0 ? (
            <div style={{ padding: 16, textAlign: 'center', color: 'var(--muted)', fontSize: 12, background: 'var(--bg)', borderRadius: 8 }}>
              Existing users are kept. Add a user to create it or change its password.
            </div>
          ) : (
            users.map((u, i) => (
              <div key={i} style={{ display: 'flex', gap: 8, marginBottom: 8 }}>
                <input value={u.username} onChange={e => { const up = [...users]; up[i] = { ...up[i], username: e.target.value }; setUsers(up) }} style={{ ...inputStyle, fontSize: 12 }} placeholder="username" />
                <input type="password" value={u.password} onChange={e => { const up = [...users]; up[i] = { ...up[i], password: e.target.value }; setUsers(up) }} style={{ ...inputStyle, fontSize: 12 }} placeholder="password" />
                <button type="button" onClick={() => setUsers(users.filter((_, j) => j !== i))} style={{ ...btnStyle, fontSize: 11, padding: '4px 8px', borderColor: 'var(--crit)', color: 'var(--crit)' }}>X</button>
              </div>
            ))
          )}
        </div>
      )}

      {updateMut.error && <div style={{ color: 'var(--crit)', fontSize: 12 }}>{(updateMut.error as Error).message}</div>}

      <div style={{ display: 'flex', gap: 8, justifyContent: 'flex-end' }}>
        <button type="button" style={btnStyle} onClick={onClose}>Cancel</button>
        <button type="submit" style={{ ...btnStyle, borderColor: 'var(--green)', color: 'var(--green)' }} disabled={updateMut.isPending}>
          {updateMut.isPending ? 'Saving...' : 'Save'}
        </button>
      </div>
    </form>
  )
}

// ── Shared styles ────────────────────────────────────────────────

const cardStyle: React.CSSProperties = {
  background: 'var(--card)',
  border: '1px solid var(--border)',
  borderRadius: 10,
  padding: 20,
}

const thStyle: React.CSSProperties = {
  textAlign: 'left', padding: '8px 12px', fontSize: 11, color: 'var(--muted)',
  textTransform: 'uppercase', letterSpacing: '.05em', fontWeight: 600,
  borderBottom: '1px solid var(--border)',
}

const tdStyle: React.CSSProperties = {
  padding: '8px 12px', fontSize: 13, borderBottom: '1px solid var(--border)',
}

// ── Main Panel ───────────────────────────────────────────────────

export function MqttBrokerPanel({ onClose, embedded }: { onClose: () => void; embedded?: boolean }) {
  const [tab, setTab] = useState<Tab>('overview')
  const [showConfig, setShowConfig] = useState(false)
  const { data: status } = useMqttBrokerStatus()
  const isRunning = status?.status === 'running'

  const tabs: { key: Tab; label: string; disabled?: boolean }[] = [
    { key: 'overview', label: 'Overview' },
    { key: 'topics', label: 'Topics', disabled: !isRunning },
    { key: 'messages', label: 'Messages', disabled: !isRunning },
    { key: 'publish', label: 'Publish', disabled: !isRunning },
  ]

  const content = (
    <div style={{ display: 'flex', flexDirection: 'column', height: '100%' }}>
      {/* Tab bar */}
      <div style={{
        display: 'flex',
        alignItems: 'center',
        justifyContent: 'space-between',
        borderBottom: '1px solid var(--border)',
        marginBottom: 16,
        flexShrink: 0,
      }}>
        <div style={{ display: 'flex' }}>
          {tabs.map(t => (
            <button
              key={t.key}
              onClick={() => !t.disabled && setTab(t.key)}
              style={{
                ...tabStyle(tab === t.key),
                opacity: t.disabled ? 0.4 : 1,
                cursor: t.disabled ? 'not-allowed' : 'pointer',
              }}
            >
              {t.label}
            </button>
          ))}
        </div>
        <button style={{ ...btnStyle, fontSize: 12 }} onClick={() => setShowConfig(s => !s)}>
          {showConfig ? 'Hide Config' : 'Config'}
        </button>
      </div>

      {showConfig ? (
        <ConfigPanel show={showConfig} onClose={() => setShowConfig(false)} />
      ) : (
        <div style={{ flex: 1, overflow: 'auto' }}>
          {tab === 'overview' && <OverviewTab />}
          {tab === 'topics' && <TopicsTab />}
          {tab === 'messages' && <MessagesTab />}
          {tab === 'publish' && <PublishTab />}
        </div>
      )}
    </div>
  )

  if (embedded) return content

  return (
    <div style={{ position: 'fixed', inset: 0, background: 'var(--bg)', zIndex: 50, display: 'flex', flexDirection: 'column', padding: 24 }}>
      <div style={{ width: '100%', height: '100%', display: 'flex', flexDirection: 'column' }}>
        <div style={{ display: 'flex', alignItems: 'center', gap: 12, marginBottom: 16, flexShrink: 0 }}>
          <button onClick={onClose} aria-label="Close MQTT Broker settings" style={{ ...btnStyle, display: 'flex', alignItems: 'center', gap: 6 }}>
            <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2"><path d="M19 12H5M12 19l-7-7 7-7" /></svg>
            Back
          </button>
          <h1 style={{ fontSize: 20, fontWeight: 700 }}>MQTT Explorer</h1>
        </div>
        {content}
      </div>
    </div>
  )
}
