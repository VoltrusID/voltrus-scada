import { useState, useEffect, useRef, useCallback } from 'react';
import { SensorConfig, SOURCE_TYPES, CHART_TYPES, SOURCE_CATEGORY } from './types';
import { inputStyle, monoInputStyle, btnStyle, parseConfig, normalizeKeyEvent, displayShortcut } from './styles';
import { SourceConfigFields } from './SourceConfigFields';
import { HistoryDeadbandFields } from './components/HistoryDeadbandFields';
import { HistoryDeadbandBulk } from './components/HistoryDeadbandBulk';
import { applyHistoryDeadband, describeHistoryDeadband, type HistoryDeadbandForm } from './historyDeadband';
import { useUpdateSensor, useReorderSensors } from './queries';

const CELL: React.CSSProperties = {
  background: 'transparent',
  border: '1px solid transparent',
  color: 'var(--fg)',
  fontSize: 12,
  padding: '4px 6px',
  width: '100%',
  boxSizing: 'border-box',
  outline: 'none',
  borderRadius: 0,
  fontFamily: 'inherit',
};
const CELL_MONO: React.CSSProperties = { ...CELL, fontFamily: "var(--font-mono)" };
const CELL_NUM: React.CSSProperties = { ...CELL_MONO, textAlign: 'right' };

const thStyle: React.CSSProperties = {
  textAlign: 'left', padding: '6px 6px', fontSize: 10, color: 'var(--muted)',
  textTransform: 'uppercase', letterSpacing: '.05em', fontWeight: 600,
  borderBottom: '1px solid var(--border)', whiteSpace: 'nowrap', position: 'sticky', top: 0,
  background: 'var(--card)', zIndex: 1,
};
const tdStyle: React.CSSProperties = {
  padding: '0', fontSize: 12, borderBottom: '1px solid var(--border)',
};

function ShortcutCell({ shortcut, onSet, onClear }: { shortcut: string | null; onSet: (s: string) => void; onClear: () => void }) {
  const [recording, setRecording] = useState(false);
  const ref = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!recording) return;
    const handler = (e: KeyboardEvent) => {
      e.preventDefault();
      e.stopPropagation();
      if (e.key === 'Escape') { setRecording(false); return; }
      if (['Control', 'Alt', 'Shift', 'Meta'].includes(e.key)) return;
      onSet(normalizeKeyEvent(e));
      setRecording(false);
    };
    window.addEventListener('keydown', handler);
    return () => window.removeEventListener('keydown', handler);
  }, [recording, onSet]);

  // Stop recording on outside click
  useEffect(() => {
    if (!recording) return;
    const handler = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) setRecording(false);
    };
    document.addEventListener('mousedown', handler);
    return () => document.removeEventListener('mousedown', handler);
  }, [recording]);

  return (
    <div ref={ref} style={{ display: 'flex', alignItems: 'center', gap: 4, padding: '2px 4px', minWidth: 80 }} onMouseDown={e => e.stopPropagation()}>
      {shortcut ? (
        <kbd style={{
          fontFamily: "var(--font-mono)", fontSize: 10, fontWeight: 600,
          padding: '2px 5px', borderRadius: 3, background: 'var(--bg)',
          border: '1px solid var(--border)', color: 'var(--cyan)',
        }}>{displayShortcut(shortcut)}</kbd>
      ) : (
        <span style={{ fontSize: 10, color: 'var(--muted)' }}>—</span>
      )}
      <button onClick={() => setRecording(true)} style={{
        padding: '1px 5px', borderRadius: 3, fontSize: 9, cursor: 'pointer',
        border: recording ? '1px solid var(--cyan)' : '1px solid var(--border)',
        background: recording ? 'rgba(34,197,94,.1)' : 'transparent',
        color: recording ? 'var(--cyan)' : 'var(--muted)',
        whiteSpace: 'nowrap',
      }}>{recording ? '...' : 'rec'}</button>
      {shortcut && !recording && (
        <button onClick={onClear} style={{
          padding: '1px 4px', borderRadius: 3, fontSize: 9, cursor: 'pointer',
          border: '1px solid var(--crit)', background: 'transparent', color: 'var(--crit)',
        }}>x</button>
      )}
    </div>
  );
}

export function SensorManager({ sensors, onSave, onDelete, onAdd, onDuplicate, onClose, shortcuts, embedded }: {
  sensors: SensorConfig[]; onSave: (s: SensorConfig, shortcut?: string | null) => void; onDelete: (id: number) => void;
  onAdd: (s: Partial<SensorConfig>) => void; onDuplicate: (s: SensorConfig) => void; onClose: () => void; shortcuts: Record<number, string>;
  embedded?: boolean;
}) {
  const [filter, setFilter] = useState('');
  const [newRow, setNewRow] = useState<Partial<SensorConfig> & { source_config_obj?: Record<string, any> } | null>(null);
  const [configPopover, setConfigPopover] = useState<{ id: number; srcType: string; config: Record<string, any>; rect: DOMRect } | null>(null);
  const [dragId, setDragId] = useState<number | null>(null);
  const [dropTargetId, setDropTargetId] = useState<number | null>(null);
  const updateMut = useUpdateSensor();
  const reorderMut = useReorderSensors();
  const configRef = useRef<HTMLDivElement>(null);
  const [selected, setSelected] = useState<Set<number>>(new Set());
  const [bulkBusy, setBulkBusy] = useState(false);
  const [popoverHistErrors, setPopoverHistErrors] = useState<string[]>([]);

  const filtered = sensors.filter(s => s.name.toLowerCase().includes(filter) || s.key.toLowerCase().includes(filter));

  useEffect(() => {
    if (embedded) return;
    const onKey = (e: KeyboardEvent) => { if (e.key === 'Escape') onClose(); };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [onClose, embedded]);

  // Close config popover on outside click
  useEffect(() => {
    if (!configPopover) return;
    const handler = (e: MouseEvent) => {
      if (configRef.current && !configRef.current.contains(e.target as Node)) {
        setConfigPopover(null);
      }
    };
    document.addEventListener('mousedown', handler);
    return () => document.removeEventListener('mousedown', handler);
  }, [configPopover]);

  const updateField = useCallback((sensor: SensorConfig, field: string, value: any) => {
    onSave({ ...sensor, [field]: value });
  }, [onSave]);

  const startConfigEdit = (sensor: SensorConfig, btn: HTMLButtonElement) => {
    const rect = btn.getBoundingClientRect();
    setConfigPopover({
      id: sensor.id,
      srcType: sensor.source_type,
      config: { ...parseConfig(sensor.source_config), ...(sensor.source_type === 'modbus' && sensor.device_id ? { modbus_device_id: sensor.device_id } : {}) },
      rect,
    });
  };

  const toggleSelected = (id: number) => setSelected(prev => {
    const next = new Set(prev);
    if (next.has(id)) next.delete(id); else next.add(id);
    return next;
  });
  const allShownSelected = filtered.length > 0 && filtered.every(s => selected.has(s.id));
  const toggleAllShown = () => setSelected(prev => {
    const next = new Set(prev);
    if (filtered.every(s => next.has(s.id))) filtered.forEach(s => next.delete(s.id));
    else filtered.forEach(s => next.add(s.id));
    return next;
  });

  // Sequential on purpose: each save rewrites the whole tag, and the list is small.
  const applyBulk = async (form: HistoryDeadbandForm | null) => {
    setBulkBusy(true);
    try {
      for (const s of sensors.filter(t => selected.has(t.id))) {
        const cfg = applyHistoryDeadband(parseConfig(s.source_config), form);
        await onSave({ ...s, source_config: JSON.stringify(cfg) });
      }
      setSelected(new Set());
    } finally {
      setBulkBusy(false);
    }
  };

  const saveConfig = () => {
    if (!configPopover) return;
    const sensor = sensors.find(s => s.id === configPopover.id);
    if (!sensor) return;
    const { modbus_device_id, ...rest } = configPopover.config;
    const update: any = { ...sensor, source_config: JSON.stringify(rest) };
    if (sensor.source_type === 'modbus' && modbus_device_id) update.device_id = modbus_device_id;
    onSave(update);
    setConfigPopover(null);
  };

  const handleAdd = () => {
    if (newRow) return;
    setNewRow({
      key: '', name: '', unit: '', color: '#8b5cf6',
      min_val: 0, max_val: 100, warn_val: 80, crit_val: 90, base_val: 50,
      decimals: 1, sort_order: 0, source_type: 'random_walk', chart_type: 'sparkline',
      source_config: '{}', source_config_obj: {},
    });
  };

  const commitNewRow = () => {
    if (!newRow) return;
    const { source_config_obj, ...rest } = newRow;
    onAdd({ ...rest, source_config: JSON.stringify(source_config_obj ?? {}) });
    setNewRow(null);
  };

  const updateNewField = (field: string, value: any) => {
    if (!newRow) return;
    setNewRow({ ...newRow, [field]: value });
  };

  // Drag-and-drop reorder
  const handleDragStart = (id: number) => setDragId(id);
  const handleDragOver = (e: React.DragEvent, id: number) => {
    e.preventDefault();
    if (dragId !== null && dragId !== id) setDropTargetId(id);
  };
  const handleDragEnd = () => {
    if (dragId !== null && dropTargetId !== null && dragId !== dropTargetId) {
      const currentIds = filtered.map(s => s.id);
      const fromIdx = currentIds.indexOf(dragId);
      const toIdx = currentIds.indexOf(dropTargetId);
      if (fromIdx !== -1 && toIdx !== -1) {
        const reordered = [...currentIds];
        reordered.splice(fromIdx, 1);
        reordered.splice(toIdx, 0, dragId);
        const filteredSet = new Set(filtered.map(s => s.id));
        let cursor = 0;
        const newOrder = sensors.map(s => (filteredSet.has(s.id) ? reordered[cursor++] : s.id));
        reorderMut.mutate(newOrder);
      }
    }
    setDragId(null);
    setDropTargetId(null);
  };

  const numField = (sensor: SensorConfig, field: keyof SensorConfig) => (
    <input
      key={`${sensor.id}-${field}-${sensor[field]}`}
      type="number" step="any"
      defaultValue={sensor[field] as number}
      onBlur={e => {
        const v = parseFloat(e.target.value);
        if (!isNaN(v) && v !== sensor[field]) updateField(sensor, field, v);
      }}
      onKeyDown={e => {
        if (e.key === 'Enter') (e.target as HTMLInputElement).blur();
      }}
      style={CELL_NUM}
    />
  );

  const renderRow = (s: SensorConfig) => {
    const isDragOver = dropTargetId === s.id && dragId !== s.id;
    const isDragging = dragId === s.id;
    return (
      <tr
        key={s.id}
        draggable
        onDragStart={() => handleDragStart(s.id)}
        onDragOver={e => handleDragOver(e, s.id)}
        onDragEnd={handleDragEnd}
        onDragLeave={() => { if (dropTargetId === s.id) setDropTargetId(null); }}
        onMouseEnter={e => { if (!isDragging && !isDragOver) (e.currentTarget as HTMLElement).style.background = 'rgba(139,92,246,.04)'; }}
        onMouseLeave={e => { if (!isDragging && !isDragOver) (e.currentTarget as HTMLElement).style.background = ''; }}
        style={{
          opacity: isDragging ? 0.4 : (updateMut.isPending && updateMut.variables?.id === s.id ? 0.6 : 1),
          cursor: 'grab',
          borderTop: isDragOver ? '2px solid var(--cyan)' : undefined,
          borderBottom: isDragOver ? 'none' : undefined,
          background: isDragOver ? 'rgba(34,197,94,.05)' : undefined,
          transition: 'background .15s, opacity .15s',
        }}
      >
        <td style={tdStyle} onMouseDown={e => e.stopPropagation()}>
          <input type="checkbox" aria-label={`Select ${s.name}`} checked={selected.has(s.id)} onChange={() => toggleSelected(s.id)} />
        </td>
        <td style={tdStyle} onMouseDown={e => e.stopPropagation()}>
          <input type="color" value={s.color} onChange={e => updateField(s, 'color', e.target.value)} style={{ width: 28, height: 24, padding: 1, border: 'none', background: 'transparent', cursor: 'pointer' }} />
        </td>
        <td style={tdStyle} onMouseDown={e => e.stopPropagation()}>
          <input key={`name-${s.id}-${s.name}`} defaultValue={s.name} onBlur={e => { if (e.target.value !== s.name) updateField(s, 'name', e.target.value); }} onKeyDown={e => { if (e.key === 'Enter') (e.target as HTMLInputElement).blur(); }} style={CELL} />
        </td>
        <td style={tdStyle} onMouseDown={e => e.stopPropagation()}>
          <input key={`key-${s.id}-${s.key}`} defaultValue={s.key} onBlur={e => { if (e.target.value !== s.key) updateField(s, 'key', e.target.value); }} onKeyDown={e => { if (e.key === 'Enter') (e.target as HTMLInputElement).blur(); }} style={CELL_MONO} />
        </td>
        <td style={tdStyle} onMouseDown={e => e.stopPropagation()}>
          <input key={`unit-${s.id}-${s.unit}`} defaultValue={s.unit} onBlur={e => { if (e.target.value !== s.unit) updateField(s, 'unit', e.target.value); }} onKeyDown={e => { if (e.key === 'Enter') (e.target as HTMLInputElement).blur(); }} style={CELL} />
        </td>
        <td style={tdStyle} onMouseDown={e => e.stopPropagation()}>{numField(s, 'min_val')}</td>
        <td style={tdStyle} onMouseDown={e => e.stopPropagation()}>{numField(s, 'max_val')}</td>
        <td style={tdStyle} onMouseDown={e => e.stopPropagation()}>{numField(s, 'warn_val')}</td>
        <td style={tdStyle} onMouseDown={e => e.stopPropagation()}>{numField(s, 'crit_val')}</td>
        <td style={tdStyle} onMouseDown={e => e.stopPropagation()}>{numField(s, 'base_val')}</td>
        <td style={tdStyle} onMouseDown={e => e.stopPropagation()}>{numField(s, 'decimals')}</td>
        <td style={tdStyle} onMouseDown={e => e.stopPropagation()}>
          <select value={s.chart_type || 'sparkline'} onChange={e => updateField(s, 'chart_type', e.target.value)} style={{ ...CELL, cursor: 'pointer' }}>
            {CHART_TYPES.map(t => <option key={t} value={t}>{t}</option>)}
          </select>
        </td>
        <td style={tdStyle} onMouseDown={e => e.stopPropagation()}>{numField(s, 'sort_order')}</td>
        <td style={tdStyle} onMouseDown={e => e.stopPropagation()}>
          <select value={s.source_type} onChange={e => {
            const newType = e.target.value;
            onSave({ ...s, source_type: newType, source_config: '{}', device_id: '' });
          }} style={{ ...CELL, cursor: 'pointer', color: SOURCE_CATEGORY[s.source_type]?.color ?? 'var(--fg)', fontSize: 11 }}>
            <optgroup label="Real">
              <option value="modbus">modbus</option>
            </optgroup>
            <optgroup label="Simulate">
              {SOURCE_TYPES.filter(t => !['modbus','api','none'].includes(t)).map(t => <option key={t} value={t}>{t}</option>)}
            </optgroup>
            <optgroup label="Other">
              <option value="api">api</option>
              <option value="none">none</option>
            </optgroup>
          </select>
        </td>
        <td style={tdStyle}>
          <ShortcutCell
            shortcut={shortcuts[s.id] ?? null}
            onSet={sc => onSave(s, sc)}
            onClear={() => onSave(s, null)}
          />
        </td>
        <td style={tdStyle} onMouseDown={e => e.stopPropagation()}>
          <button onClick={e => startConfigEdit(s, e.currentTarget)} title={describeHistoryDeadband(parseConfig(s.source_config)) ? `History deadband: ${describeHistoryDeadband(parseConfig(s.source_config))}` : 'Source config'} style={{ padding: '2px 8px', borderRadius: 3, border: '1px solid var(--border)', background: 'transparent', color: 'var(--cyan)', cursor: 'pointer', fontSize: 10 }}>cfg{describeHistoryDeadband(parseConfig(s.source_config)) ? '*' : ''}</button>
        </td>
        <td style={{ ...tdStyle, whiteSpace: 'nowrap' }} onMouseDown={e => e.stopPropagation()}>
          <div style={{ display: 'flex', gap: 3 }}>
            <button onClick={() => onDuplicate(s)} aria-label={`Duplicate ${s.name}`} style={{ padding: '2px 6px', borderRadius: 3, border: '1px solid var(--cyan)', background: 'transparent', color: 'var(--cyan)', cursor: 'pointer', fontSize: 10 }}>dup</button>
            <button onClick={() => { if (confirm(`Delete ${s.name}?`)) onDelete(s.id); }} aria-label={`Delete ${s.name}`} style={{ padding: '2px 6px', borderRadius: 3, border: '1px solid var(--crit)', background: 'transparent', color: 'var(--crit)', cursor: 'pointer', fontSize: 10 }}>del</button>
          </div>
        </td>
      </tr>
    );
  };

  const renderNewRow = () => {
    if (!newRow) return null;
    return (
      <tr style={{ background: 'rgba(139,92,246,.05)' }}>
        <td style={tdStyle} />
        <td style={tdStyle}>
          <input type="color" value={newRow.color || '#8b5cf6'} onChange={e => updateNewField('color', e.target.value)} style={{ width: 28, height: 24, padding: 1, border: 'none', background: 'transparent', cursor: 'pointer' }} />
        </td>
        <td style={tdStyle}><input value={newRow.name ?? ''} onChange={e => updateNewField('name', e.target.value)} placeholder="Name" style={CELL} /></td>
        <td style={tdStyle}><input value={newRow.key ?? ''} onChange={e => updateNewField('key', e.target.value)} placeholder="key" style={CELL_MONO} /></td>
        <td style={tdStyle}><input value={newRow.unit ?? ''} onChange={e => updateNewField('unit', e.target.value)} placeholder="V" style={CELL} /></td>
        <td style={tdStyle}><input type="number" step="any" value={newRow.min_val ?? 0} onChange={e => updateNewField('min_val', parseFloat(e.target.value) || 0)} style={CELL_NUM} /></td>
        <td style={tdStyle}><input type="number" step="any" value={newRow.max_val ?? 100} onChange={e => updateNewField('max_val', parseFloat(e.target.value) || 0)} style={CELL_NUM} /></td>
        <td style={tdStyle}><input type="number" step="any" value={newRow.warn_val ?? 80} onChange={e => updateNewField('warn_val', parseFloat(e.target.value) || 0)} style={CELL_NUM} /></td>
        <td style={tdStyle}><input type="number" step="any" value={newRow.crit_val ?? 90} onChange={e => updateNewField('crit_val', parseFloat(e.target.value) || 0)} style={CELL_NUM} /></td>
        <td style={tdStyle}><input type="number" step="any" value={newRow.base_val ?? 50} onChange={e => updateNewField('base_val', parseFloat(e.target.value) || 0)} style={CELL_NUM} /></td>
        <td style={tdStyle}><input type="number" value={newRow.decimals ?? 1} onChange={e => updateNewField('decimals', parseInt(e.target.value) || 0)} style={CELL_NUM} /></td>
        <td style={tdStyle}>
          <select value={newRow.chart_type ?? 'sparkline'} onChange={e => updateNewField('chart_type', e.target.value)} style={{ ...CELL, cursor: 'pointer' }}>
            {CHART_TYPES.map(t => <option key={t} value={t}>{t}</option>)}
          </select>
        </td>
        <td style={tdStyle}><input type="number" value={newRow.sort_order ?? 0} onChange={e => updateNewField('sort_order', parseInt(e.target.value) || 0)} style={CELL_NUM} /></td>
        <td style={tdStyle}>
          <select value={newRow.source_type ?? 'random_walk'} onChange={e => updateNewField('source_type', e.target.value)} style={{ ...CELL, cursor: 'pointer', fontSize: 11 }}>
            <optgroup label="Real"><option value="modbus">modbus</option></optgroup>
            <optgroup label="Simulate">{SOURCE_TYPES.filter(t => !['modbus','api','none'].includes(t)).map(t => <option key={t} value={t}>{t}</option>)}</optgroup>
            <optgroup label="Other"><option value="api">api</option><option value="none">none</option></optgroup>
          </select>
        </td>
        <td style={tdStyle} />
        <td style={tdStyle} />
        <td style={{ ...tdStyle, whiteSpace: 'nowrap' }}>
          <div style={{ display: 'flex', gap: 3 }}>
            <button onClick={commitNewRow} disabled={!(newRow?.key ?? '').trim()} style={{ padding: '2px 8px', borderRadius: 3, border: '1px solid var(--green)', background: 'transparent', color: 'var(--green)', cursor: (newRow?.key ?? '').trim() ? 'pointer' : 'not-allowed', opacity: (newRow?.key ?? '').trim() ? 1 : 0.5, fontSize: 10 }}>ok</button>
            <button onClick={() => setNewRow(null)} style={{ padding: '2px 6px', borderRadius: 3, border: '1px solid var(--border)', background: 'transparent', color: 'var(--muted)', cursor: 'pointer', fontSize: 10 }}>x</button>
          </div>
        </td>
      </tr>
    );
  };

  const tableContent = (
    <>
      <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', marginBottom: 16, flexShrink: 0 }}>
        <span style={{ color: 'var(--muted)', fontSize: 13 }}>{sensors.length} sources</span>
        <div style={{ display: 'flex', gap: 8 }}>
          <input placeholder="Search..." value={filter} onChange={e => setFilter(e.target.value)} style={{ ...inputStyle, width: 200 }} />
          <button style={{ ...btnStyle, borderColor: 'var(--green)', color: 'var(--green)' }} onClick={handleAdd} aria-label="Create new data source">+ Create Source</button>
        </div>
      </div>
      {selected.size > 0 && (
        <HistoryDeadbandBulk count={selected.size} busy={bulkBusy} onApply={applyBulk} onClearSelection={() => setSelected(new Set())} />
      )}
      <div style={{ flex: 1, overflow: 'auto', background: 'var(--card)', border: '1px solid var(--border)', borderRadius: 10 }}>
        <table style={{ width: '100%', borderCollapse: 'collapse', minWidth: 1200 }}>
          <colgroup>
            <col style={{ width: 28 }} />
            <col style={{ width: 40 }} />
            <col style={{ width: 140 }} />
            <col style={{ width: 120 }} />
            <col style={{ width: 60 }} />
            <col style={{ width: 64 }} />
            <col style={{ width: 64 }} />
            <col style={{ width: 64 }} />
            <col style={{ width: 64 }} />
            <col style={{ width: 64 }} />
            <col style={{ width: 40 }} />
            <col style={{ width: 90 }} />
            <col style={{ width: 40 }} />
            <col style={{ width: 90 }} />
            <col style={{ width: 100 }} />
            <col style={{ width: 60 }} />
            <col style={{ width: 90 }} />
          </colgroup>
          <thead>
            <tr>
              <th style={thStyle}>
                <input type="checkbox" aria-label="Select all shown tags" checked={allShownSelected} onChange={toggleAllShown} />
              </th>
              <th style={thStyle}></th>
              <th style={thStyle}>name</th>
              <th style={thStyle}>key</th>
              <th style={thStyle}>unit</th>
              <th style={thStyle}>min</th>
              <th style={thStyle}>max</th>
              <th style={thStyle}>warn</th>
              <th style={thStyle}>crit</th>
              <th style={thStyle}>base</th>
              <th style={thStyle}>dec</th>
              <th style={thStyle}>chart</th>
              <th style={thStyle}>sort</th>
              <th style={thStyle}>source</th>
              <th style={thStyle}>hotkey</th>
              <th style={thStyle}>cfg</th>
              <th style={thStyle}></th>
            </tr>
          </thead>
          <tbody>
            {renderNewRow()}
            {filtered.map(s => renderRow(s))}
          </tbody>
        </table>
      </div>
    </>
  );

  const configPopoverEl = configPopover && (
    <div
      ref={configRef}
      style={{
        position: 'fixed',
        left: Math.min(configPopover.rect.right + 4, window.innerWidth - 380),
        top: Math.min(configPopover.rect.top, window.innerHeight - 300),
        background: 'var(--card)',
        border: '1px solid var(--border)',
        borderRadius: 8,
        padding: 12,
        width: 340,
        maxHeight: '70vh',
        overflow: 'auto',
        zIndex: 70,
        boxShadow: '0 8px 24px rgba(0,0,0,.4)',
      }}
      onClick={e => e.stopPropagation()}
    >
      <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', marginBottom: 8 }}>
        <span style={{ fontSize: 12, fontWeight: 600 }}>Source Config</span>
        <button onClick={saveConfig} disabled={popoverHistErrors.length > 0} title={popoverHistErrors[0]} aria-label="Save source config" style={{ padding: '3px 10px', borderRadius: 4, border: '1px solid var(--cyan)', background: 'transparent', color: 'var(--cyan)', cursor: popoverHistErrors.length > 0 ? 'not-allowed' : 'pointer', opacity: popoverHistErrors.length > 0 ? 0.5 : 1, fontSize: 11 }}>Save</button>
      </div>
      <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 8 }}>
        <SourceConfigFields
          sourceType={configPopover.srcType}
          config={configPopover.config}
          onChange={c => setConfigPopover({ ...configPopover, config: c })}
        />
      </div>
      <div style={{ borderTop: '1px solid var(--border)', marginTop: 10, paddingTop: 8 }}>
        <div style={{ fontSize: 10, fontWeight: 700, color: 'var(--muted)', textTransform: 'uppercase', letterSpacing: '.05em', marginBottom: 6 }}>History storage (deadband, optional)</div>
        <HistoryDeadbandFields
          key={configPopover.id}
          config={configPopover.config}
          onChange={c => setConfigPopover(p => (p ? { ...p, config: c } : p))}
          onValidityChange={setPopoverHistErrors}
        />
      </div>
    </div>
  );

  if (embedded) {
    return (<>{tableContent}{configPopoverEl}</>);
  }

  return (
    <div style={{ position: 'fixed', inset: 0, background: 'var(--bg)', zIndex: 50, display: 'flex', flexDirection: 'column', padding: 24 }}>
      <div style={{ width: '100%', height: '100%', display: 'flex', flexDirection: 'column' }}>
        <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', marginBottom: 16, flexShrink: 0 }}>
          <div style={{ display: 'flex', alignItems: 'center', gap: 12 }}>
            <button onClick={onClose} aria-label="Close data sources" style={{ ...btnStyle, display: 'flex', alignItems: 'center', gap: 6 }}>
              <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2"><path d="M19 12H5M12 19l-7-7 7-7" /></svg>Back
            </button>
            <h1 style={{ fontSize: 20, fontWeight: 700 }}>Data Sources</h1>
          </div>
        </div>
        {tableContent}
      </div>
      {configPopoverEl}
    </div>
  );
}
