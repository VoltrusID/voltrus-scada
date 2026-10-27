import { useState, useMemo, useEffect } from 'react';
import { useAuditLogs, useExportAuditCsv, type AuditQueryParams } from './queries';
import { inputStyle, btnStyle } from './styles';
import { OverlayScreen } from './components/OverlayScreen';
import { auditRangeParams } from './unixTime';

const LIMIT = 25;

export function AuditLogPanel({ onClose }: { onClose?: () => void } = {}) {
  const [search, setSearch] = useState('');
  const [action, setAction] = useState('');
  const [username, setUsername] = useState('');
  const [from, setFrom] = useState('');
  const [to, setTo] = useState('');
  const [page, setPage] = useState(1);

  const params: AuditQueryParams = useMemo(() => {
    const p: AuditQueryParams = { page, limit: LIMIT };
    if (search) p.search = search;
    if (action) p.action = action;
    if (username) p.username = username;
    Object.assign(p, auditRangeParams(from, to));
    return p;
  }, [search, action, username, from, to, page]);

  const { data, isLoading } = useAuditLogs(params);
  const exportMut = useExportAuditCsv();

  const entries = data?.entries ?? [];
  const totalPages = data?.total_pages ?? 1;

  const handleExport = () => {
    const exportParams: AuditQueryParams = {};
    if (search) exportParams.search = search;
    if (action) exportParams.action = action;
    if (username) exportParams.username = username;
    Object.assign(exportParams, auditRangeParams(from, to));
    exportMut.mutate(exportParams, {
      onSuccess: (csv) => {
        const blob = new Blob([csv], { type: 'text/csv' });
        const url = URL.createObjectURL(blob);
        const a = document.createElement('a');
        a.href = url;
        a.download = `audit-log-${new Date().toISOString().slice(0, 10)}.csv`;
        a.click();
        URL.revokeObjectURL(url);
      },
    });
  };

  const thStyle: React.CSSProperties = {
    textAlign: 'left', padding: '8px 10px', fontSize: 10, color: 'var(--muted)',
    textTransform: 'uppercase', letterSpacing: '.05em', fontWeight: 600,
    borderBottom: '1px solid var(--border)', whiteSpace: 'nowrap', position: 'sticky', top: 0,
    background: 'var(--card)', zIndex: 1,
  };
  const tdStyle: React.CSSProperties = {
    padding: '8px 10px', fontSize: 12, borderBottom: '1px solid var(--border)', color: 'var(--fg)',
  };

  const content = (
    <div style={{ display: 'flex', flexDirection: 'column', height: '100%', gap: 12 }}>
      {/* Filters */}
      <div style={{ display: 'flex', gap: 8, flexWrap: 'wrap', alignItems: 'flex-end' }}>
        <div style={{ flex: 1, minWidth: 140 }}>
          <label style={{ fontSize: 11, color: 'var(--muted)', textTransform: 'uppercase', display: 'block', marginBottom: 4 }}>Search</label>
          <input placeholder="Search..." value={search} onChange={e => { setSearch(e.target.value); setPage(1); }} style={inputStyle} />
        </div>
        <div style={{ minWidth: 120 }}>
          <label style={{ fontSize: 11, color: 'var(--muted)', textTransform: 'uppercase', display: 'block', marginBottom: 4 }}>Action</label>
          <input placeholder="e.g. create" value={action} onChange={e => { setAction(e.target.value); setPage(1); }} style={inputStyle} />
        </div>
        <div style={{ minWidth: 120 }}>
          <label style={{ fontSize: 11, color: 'var(--muted)', textTransform: 'uppercase', display: 'block', marginBottom: 4 }}>User</label>
          <input placeholder="Username" value={username} onChange={e => { setUsername(e.target.value); setPage(1); }} style={inputStyle} />
        </div>
        <div style={{ minWidth: 140 }}>
          <label style={{ fontSize: 11, color: 'var(--muted)', textTransform: 'uppercase', display: 'block', marginBottom: 4 }}>From</label>
          <input type="datetime-local" value={from} onChange={e => { setFrom(e.target.value); setPage(1); }} style={inputStyle} />
        </div>
        <div style={{ minWidth: 140 }}>
          <label style={{ fontSize: 11, color: 'var(--muted)', textTransform: 'uppercase', display: 'block', marginBottom: 4 }}>To</label>
          <input type="datetime-local" value={to} onChange={e => { setTo(e.target.value); setPage(1); }} style={inputStyle} />
        </div>
        <button onClick={handleExport} disabled={exportMut.isPending} style={{ ...btnStyle, whiteSpace: 'nowrap' }}>
          {exportMut.isPending ? 'Exporting...' : 'Export CSV'}
        </button>
      </div>

      {/* Table */}
      <div style={{ flex: 1, overflow: 'auto', background: 'var(--card)', border: '1px solid var(--border)', borderRadius: 10 }}>
        <table style={{ width: '100%', borderCollapse: 'collapse' }}>
          <thead>
            <tr>
              <th style={thStyle}>Timestamp</th>
              <th style={thStyle}>Action</th>
              <th style={thStyle}>User</th>
              <th style={thStyle}>Resource</th>
              <th style={thStyle}>Details</th>
            </tr>
          </thead>
          <tbody>
            {isLoading ? (
              <tr><td colSpan={5} style={{ ...tdStyle, color: 'var(--muted)' }}>Loading...</td></tr>
            ) : entries.length === 0 ? (
              <tr><td colSpan={5} style={{ ...tdStyle, color: 'var(--muted)' }}>No entries found.</td></tr>
            ) : (
              entries.map(entry => (
                <tr key={entry.id} style={{ transition: 'background .15s' }} onMouseEnter={e => { (e.currentTarget as HTMLElement).style.background = 'rgba(139,92,246,.04)'; }} onMouseLeave={e => { (e.currentTarget as HTMLElement).style.background = ''; }}>
                  <td style={tdStyle}>{new Date(entry.created_at * 1000).toLocaleString()}</td>
                  <td style={tdStyle}><span style={{ fontSize: 10, padding: '2px 8px', borderRadius: 4, background: 'var(--bg)', fontWeight: 600 }}>{entry.action}</span></td>
                  <td style={tdStyle}>{entry.username}</td>
                  <td style={tdStyle}>{entry.resource}</td>
                  <td style={tdStyle}>{entry.details ?? '—'}</td>
                </tr>
              ))
            )}
          </tbody>
        </table>
      </div>

      {/* Pagination */}
      <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between' }}>
        <span style={{ fontSize: 12, color: 'var(--muted)' }}>
          Page {page} of {totalPages} {data ? `(${data.total} total)` : ''}
        </span>
        <div style={{ display: 'flex', gap: 8 }}>
          <button onClick={() => setPage(p => Math.max(1, p - 1))} disabled={page <= 1} style={{ ...btnStyle }}>Prev</button>
          <button onClick={() => setPage(p => Math.min(totalPages, p + 1))} disabled={page >= totalPages} style={{ ...btnStyle }}>Next</button>
        </div>
      </div>
    </div>
  );

  // When used standalone (from header menu), wrap in OverlayScreen
  if (onClose) {
    return <OverlayScreen title="Audit Log" onClose={onClose}>{content}</OverlayScreen>;
  }

  // When embedded (e.g. inside DataManager tab), render content directly
  return content;
}
