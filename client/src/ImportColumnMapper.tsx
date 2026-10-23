import { useState } from 'react';
import { btnStyle } from './styles';

interface ColumnMapping {
  tag_name: number | null;
  value: number | null;
  timestamp: number | null;
  device_id: number | null;
}

interface ImportColumnMapperProps {
  headers: string[];
  detectedMapping: Record<string, number>;
  onConfirm: (mapping: ColumnMapping) => void;
  onCancel: () => void;
}

const VOLTRUS_FIELDS: { key: keyof ColumnMapping; label: string; required: boolean; description: string }[] = [
  { key: 'tag_name', label: 'Tag Name', required: true, description: 'Unique tag identifier (required)' },
  { key: 'value', label: 'Value', required: false, description: 'Numeric or string value' },
  { key: 'timestamp', label: 'Timestamp', required: false, description: 'ISO 8601 or Unix timestamp' },
  { key: 'device_id', label: 'Device ID', required: false, description: 'Device or data source identifier' },
];

export function ImportColumnMapper({ headers, detectedMapping, onConfirm, onCancel }: ImportColumnMapperProps) {
  const buildInitial = (): ColumnMapping => {
    const m: ColumnMapping = { tag_name: null, value: null, timestamp: null, device_id: null };
    for (const [key, idx] of Object.entries(detectedMapping)) {
      if (key in m) {
        (m as unknown as Record<string, number | null>)[key] = idx;
      }
    }
    return m;
  };

  const [mapping, setMapping] = useState<ColumnMapping>(buildInitial);

  const handleChange = (field: keyof ColumnMapping, value: string) => {
    const idx = value === '' ? null : Number(value);
    setMapping(prev => ({ ...prev, [field]: idx }));
  };

  const tagMapped = mapping.tag_name !== null;

  const // Check if a column index is already used by another field
  getConflict = (field: keyof ColumnMapping, idx: number | null): string | null => {
    if (idx === null) return null;
    for (const f of VOLTRUS_FIELDS) {
      if (f.key !== field && mapping[f.key] === idx) {
        return `Already used by ${f.label}`;
      }
    }
    return null;
  };

  const hasConflict = VOLTRUS_FIELDS.some(f => getConflict(f.key, mapping[f.key]) !== null);
  const canConfirm = tagMapped && !hasConflict;

  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 16 }}>
      <div style={{ padding: 16, background: 'var(--card)', borderRadius: 10, border: '1px solid var(--border)' }}>
        <div style={{ fontSize: 14, fontWeight: 700, marginBottom: 4 }}>Column Mapping</div>
        <div style={{ fontSize: 12, color: 'var(--muted)', marginBottom: 16 }}>
          Map your file columns to Voltrus fields. At minimum, a Tag Name column is required.
        </div>

        <table style={{ width: '100%', borderCollapse: 'collapse' }}>
          <thead>
            <tr>
              <th style={{ textAlign: 'left', fontSize: 11, color: 'var(--muted)', textTransform: 'uppercase', padding: '8px 12px', borderBottom: '1px solid var(--border)' }}>Voltrus Field</th>
              <th style={{ textAlign: 'left', fontSize: 11, color: 'var(--muted)', textTransform: 'uppercase', padding: '8px 12px', borderBottom: '1px solid var(--border)' }}>File Column</th>
              <th style={{ textAlign: 'left', fontSize: 11, color: 'var(--muted)', textTransform: 'uppercase', padding: '8px 12px', borderBottom: '1px solid var(--border)', width: 80 }}>Required</th>
            </tr>
          </thead>
          <tbody>
            {VOLTRUS_FIELDS.map(field => {
              const currentValue = mapping[field.key];
              const conflict = getConflict(field.key, currentValue);
              const wasDetected = detectedMapping[field.key] !== undefined;
              return (
                <tr key={field.key}>
                  <td style={{ padding: '10px 12px', borderBottom: '1px solid var(--border)' }}>
                    <div>
                      <span style={{ fontSize: 13, fontWeight: 600, color: 'var(--fg)' }}>{field.label}</span>
                      <div style={{ fontSize: 11, color: 'var(--muted)', marginTop: 2 }}>{field.description}</div>
                    </div>
                  </td>
                  <td style={{ padding: '10px 12px', borderBottom: '1px solid var(--border)' }}>
                    <select
                      value={currentValue === null ? '' : String(currentValue)}
                      onChange={e => handleChange(field.key, e.target.value)}
                      style={{
                        width: '100%',
                        padding: '6px 8px',
                        borderRadius: 4,
                        border: `1px solid ${conflict ? '#ef4444' : wasDetected && currentValue !== null ? 'var(--cyan)' : 'var(--border)'}`,
                        background: 'var(--bg)',
                        color: 'var(--fg)',
                        fontSize: 13,
                        fontFamily: 'inherit',
                        cursor: 'pointer',
                      }}
                    >
                      <option value="">-- not mapped --</option>
                      {headers.map((h, i) => (
                        <option key={i} value={String(i)}>{h}</option>
                      ))}
                    </select>
                    {conflict && (
                      <div style={{ fontSize: 11, color: '#ef4444', marginTop: 4 }}>{conflict}</div>
                    )}
                    {wasDetected && currentValue !== null && !conflict && (
                      <div style={{ fontSize: 11, color: 'var(--cyan)', marginTop: 4 }}>Auto-detected</div>
                    )}
                  </td>
                  <td style={{ padding: '10px 12px', borderBottom: '1px solid var(--border)', textAlign: 'center' }}>
                    {field.required ? (
                      <span style={{ fontSize: 11, fontWeight: 700, color: tagMapped ? 'var(--green)' : '#ef4444' }}>
                        {tagMapped ? 'SET' : 'MISSING'}
                      </span>
                    ) : (
                      <span style={{ fontSize: 11, color: 'var(--muted)' }}>Optional</span>
                    )}
                  </td>
                </tr>
              );
            })}
          </tbody>
        </table>

        {!tagMapped && (
          <div style={{ marginTop: 12, padding: '10px 12px', background: 'rgba(239,68,68,.08)', borderRadius: 6, color: '#ef4444', fontSize: 12, fontWeight: 600 }}>
            Tag Name column must be mapped before proceeding with import.
          </div>
        )}

        {headers.length > 0 && (
          <div style={{ marginTop: 12 }}>
            <div style={{ fontSize: 11, color: 'var(--muted)', textTransform: 'uppercase', marginBottom: 6 }}>Detected File Columns ({headers.length})</div>
            <div style={{ display: 'flex', flexWrap: 'wrap', gap: 6 }}>
              {headers.map((h, i) => (
                <span key={i} style={{ fontSize: 11, padding: '3px 8px', background: 'var(--bg)', borderRadius: 4, border: '1px solid var(--border)', color: 'var(--fg)' }}>
                  {i}: {h}
                </span>
              ))}
            </div>
          </div>
        )}
      </div>

      <div style={{ display: 'flex', gap: 8 }}>
        <button onClick={onCancel} style={btnStyle}>
          Back
        </button>
        <button
          onClick={() => onConfirm(mapping)}
          disabled={!canConfirm}
          style={{
            ...btnStyle,
            borderColor: 'var(--cyan)',
            color: canConfirm ? 'var(--cyan)' : 'var(--muted)',
            opacity: canConfirm ? 1 : 0.5,
            cursor: canConfirm ? 'pointer' : 'not-allowed',
          }}
        >
          Confirm Mapping
        </button>
      </div>
    </div>
  );
}

export type { ColumnMapping };
