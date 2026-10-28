import { useState, useEffect, useRef } from 'react';
import { SensorConfig } from './types';
import { SOURCE_TYPES, CHART_TYPES, SOURCE_CATEGORY, SIMULATE_TYPES } from './types';
import { inputStyle, monoInputStyle, labelStyle, labelTextStyle, tabStyle, parseConfig, normalizeKeyEvent, displayShortcut } from './styles';
import { SourceConfigFields } from './SourceConfigFields';
import { HistoryDeadbandFields } from './components/HistoryDeadbandFields';

export function EditPanel({ sensor, activeScreen, onSave, onDelete, onClose, shortcuts }: { sensor: SensorConfig; activeScreen: string; onSave: (s: SensorConfig, shortcut?: string | null) => void; onDelete: (id: number) => void; onClose: () => void; shortcuts: Record<number, string> }) {
  const [form, setForm] = useState(sensor);
  const [tab, setTab] = useState<'general' | 'source'>('general');
  const [histErrors, setHistErrors] = useState<string[]>([]);
  const [srcConfig, setSrcConfig] = useState<Record<string, any>>(() => {
    const parsed = parseConfig(sensor.source_config);
    if (sensor.source_type === 'modbus' && sensor.device_id) {
      parsed.modbus_device_id = sensor.device_id;
    }
    return parsed;
  });
  const [recording, setRecording] = useState(false);
  const [capturedShortcut, setCapturedShortcut] = useState<string | null>(shortcuts[sensor.id] ?? null);
  const [imageTab, setImageTab] = useState<'url' | 'upload'>('upload');
  const fileRef = useRef<HTMLInputElement>(null);
  const isImage = (form.chart_type || 'sparkline') === 'image';
  const isText = (form.chart_type || 'sparkline') === 'text';
  const hideNumberFields = isImage || isText;

  useEffect(() => {
    if (!recording) return;
    const onKey = (e: KeyboardEvent) => {
      e.preventDefault();
      e.stopPropagation();
      if (e.key === 'Escape') {
        setCapturedShortcut(null);
        setRecording(false);
        return;
      }
      if (['Control', 'Alt', 'Shift', 'Meta'].includes(e.key)) return;
      setCapturedShortcut(normalizeKeyEvent(e));
      setRecording(false);
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [recording]);

  return (
    <div style={{ position: 'fixed', inset: 0, background: 'rgba(0,0,0,.6)', display: 'flex', alignItems: 'center', justifyContent: 'center', zIndex: 60 }} onClick={onClose}>
      <div style={{ background: 'var(--card)', border: '1px solid var(--border)', borderRadius: 10, width: 480, maxHeight: '80vh', display: 'flex', flexDirection: 'column' }} onClick={e => e.stopPropagation()}>
        <div style={{ overflow: 'auto', flex: 1 }}>
        <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', padding: '12px 16px', borderBottom: '1px solid var(--border)' }}>
          <div style={{ fontSize: 13, fontWeight: 600 }}>Edit: {sensor.name}</div>
          <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
            <div style={{ fontSize: 10, padding: '2px 8px', borderRadius: 4, background: `${SOURCE_CATEGORY[form.source_type]?.color ?? 'var(--muted)'}18`, color: SOURCE_CATEGORY[form.source_type]?.color ?? 'var(--muted)', fontWeight: 700, letterSpacing: '.04em' }}>{SOURCE_CATEGORY[form.source_type]?.tag ?? form.source_type} — {SOURCE_CATEGORY[form.source_type]?.label ?? form.source_type}</div>
            <button onClick={onClose} style={{ background: 'none', border: 'none', color: 'var(--muted)', cursor: 'pointer', fontSize: 18, lineHeight: 1 }}>&times;</button>
          </div>
        </div>
        <div style={{ display: 'flex', borderBottom: '1px solid var(--border)' }}>
          {(['general', 'source'] as const).map(t => (<button key={t} style={tabStyle(tab === t)} onClick={() => setTab(t)}>{t}</button>))}
        </div>
        <div style={{ padding: 16 }}>
          {tab === 'general' && (<div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 8 }}>
            {(['name', 'key'] as const).map(f => (<label key={f} style={labelStyle}><span style={labelTextStyle}>{f}</span><input value={form[f]} onChange={e => setForm({ ...form, [f]: e.target.value })} style={inputStyle} /></label>))}
            {!hideNumberFields && (['unit'] as const).map(f => (<label key={f} style={labelStyle}><span style={labelTextStyle}>{f}</span><input value={form[f]} onChange={e => setForm({ ...form, [f]: e.target.value })} style={inputStyle} /></label>))}
            {!hideNumberFields && (['min_val', 'max_val', 'warn_val', 'crit_val', 'base_val', 'decimals'] as const).map(f => (<label key={f} style={labelStyle}><span style={labelTextStyle}>{f.replace('_', ' ')}</span><input type="number" step="any" value={form[f]} onChange={e => setForm({ ...form, [f]: parseFloat(e.target.value) || 0 })} style={monoInputStyle} /></label>))}
            {!hideNumberFields && (
              <div style={{ gridColumn: '1 / -1', borderTop: '1px solid var(--border)', marginTop: 4, paddingTop: 8 }}>
                <div style={{ fontSize: 10, fontWeight: 700, color: 'var(--muted)', textTransform: 'uppercase', letterSpacing: '.05em', marginBottom: 6 }}>Low-alarm limits &amp; deadband (optional)</div>
                <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr 1fr', gap: 8 }}>
                  <label style={labelStyle}><span style={labelTextStyle}>low warn</span><input type="number" step="any" value={srcConfig.low_warn ?? ''} onChange={e => setSrcConfig(c => { const n = { ...c }; const v = e.target.value === '' ? NaN : parseFloat(e.target.value); if (isNaN(v)) delete n.low_warn; else n.low_warn = v; return n; })} placeholder="—" style={monoInputStyle} /></label>
                  <label style={labelStyle}><span style={labelTextStyle}>low crit</span><input type="number" step="any" value={srcConfig.low_crit ?? ''} onChange={e => setSrcConfig(c => { const n = { ...c }; const v = e.target.value === '' ? NaN : parseFloat(e.target.value); if (isNaN(v)) delete n.low_crit; else n.low_crit = v; return n; })} placeholder="—" style={monoInputStyle} /></label>
                  <label style={labelStyle}><span style={labelTextStyle}>deadband</span><input type="number" step="any" min="0" value={srcConfig.deadband ?? 0} onChange={e => setSrcConfig(c => ({ ...c, deadband: parseFloat(e.target.value) || 0 }))} placeholder="0" style={monoInputStyle} /></label>
                </div>
              </div>
            )}
            {!hideNumberFields && (
              <div style={{ gridColumn: '1 / -1', borderTop: '1px solid var(--border)', marginTop: 4, paddingTop: 8 }}>
                <div style={{ fontSize: 10, fontWeight: 700, color: 'var(--muted)', textTransform: 'uppercase', letterSpacing: '.05em', marginBottom: 6 }}>History storage (deadband, optional)</div>
                <HistoryDeadbandFields config={srcConfig} onChange={setSrcConfig} onValidityChange={setHistErrors} />
              </div>
            )}
            {!hideNumberFields && (
              <div style={{ gridColumn: '1 / -1', borderTop: '1px solid var(--border)', marginTop: 4, paddingTop: 8 }}>
                <div style={{ fontSize: 10, fontWeight: 700, color: 'var(--muted)', textTransform: 'uppercase', letterSpacing: '.05em', marginBottom: 6 }}>ISA-18.2 alarm delays (seconds)</div>
                <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 8 }}>
                  <label style={labelStyle}><span style={labelTextStyle}>on delay (s)</span><input type="number" min="0" step="1" value={form.on_delay_secs ?? 0} onChange={e => setForm({ ...form, on_delay_secs: parseFloat(e.target.value) || 0 })} placeholder="0" style={monoInputStyle} /></label>
                  <label style={labelStyle}><span style={labelTextStyle}>off delay (s)</span><input type="number" min="0" step="1" value={form.off_delay_secs ?? 0} onChange={e => setForm({ ...form, off_delay_secs: parseFloat(e.target.value) || 0 })} placeholder="0" style={monoInputStyle} /></label>
                </div>
              </div>
            )}
            <label style={labelStyle}><span style={labelTextStyle}>color</span><input type="color" value={form.color} onChange={e => setForm({ ...form, color: e.target.value })} style={{ height: 32, padding: 2, background: 'var(--bg)', border: '1px solid var(--border)', borderRadius: 6 }} /></label>
            <label style={labelStyle}><span style={labelTextStyle}>chart type</span><select value={form.chart_type || 'sparkline'} onChange={e => setForm({ ...form, chart_type: e.target.value })} style={inputStyle}>{CHART_TYPES.map(t => <option key={t} value={t}>{t}</option>)}</select></label>
            {(form.chart_type || 'sparkline') === 'image' && (
              <div style={{ gridColumn: '1 / -1' }}>
                <span style={labelTextStyle}>Image</span>
                <div style={{ display: 'flex', gap: 0, borderBottom: '1px solid var(--border)', marginBottom: 8, marginTop: 4 }}>
                  {(['url', 'upload'] as const).map(tab => (
                    <button key={tab} onClick={() => setImageTab(tab)} style={{
                      padding: '4px 12px', fontSize: 10, fontWeight: 600, cursor: 'pointer', border: 'none',
                      borderBottom: imageTab === tab ? '2px solid var(--cyan)' : '2px solid transparent',
                      color: imageTab === tab ? 'var(--cyan)' : 'var(--muted)', background: 'transparent',
                      textTransform: 'uppercase' as const, letterSpacing: '.04em',
                    }}>{tab === 'url' ? 'URL' : 'Upload'}</button>
                  ))}
                </div>
                {imageTab === 'url' ? (
                  <input value={(srcConfig.image_url && !srcConfig.image_url.startsWith('data:')) ? srcConfig.image_url : ''} onChange={e => setSrcConfig({ ...srcConfig, image_url: e.target.value })} placeholder="https://example.com/image.png" style={inputStyle} />
                ) : (
                  <div>
                    <input ref={fileRef} type="file" accept="image/*" onChange={e => {
                      const file = e.target.files?.[0]; if (!file) return;
                      if (file.size > 200_000) return;
                      const reader = new FileReader();
                      reader.onload = () => { if (typeof reader.result === 'string') setSrcConfig({ ...srcConfig, image_url: reader.result }); };
                      reader.readAsDataURL(file);
                    }} style={{ display: 'none' }} />
                    <button onClick={() => fileRef.current?.click()} style={{ padding: '5px 12px', borderRadius: 6, border: '1px solid var(--border)', background: 'var(--bg)', color: 'var(--fg)', cursor: 'pointer', fontSize: 12 }}>Choose File</button>
                  </div>
                )}
                {srcConfig.image_url && (
                  <div style={{ display: 'flex', alignItems: 'center', gap: 8, marginTop: 8 }}>
                    <img src={srcConfig.image_url} alt="" style={{ width: 40, height: 40, borderRadius: 4, objectFit: 'contain', border: '1px solid var(--border)' }} />
                    <button onClick={() => setSrcConfig({ ...srcConfig, image_url: '' })} style={{ fontSize: 10, color: 'var(--crit)', background: 'none', border: 'none', cursor: 'pointer' }}>Remove</button>
                  </div>
                )}
              </div>
            )}
            {isText && (
              <div style={{ gridColumn: '1 / -1' }}>
                <span style={labelTextStyle}>Content (Markdown)</span>
                <textarea
                  value={srcConfig.text_content || ''}
                  onChange={e => setSrcConfig({ ...srcConfig, text_content: e.target.value })}
                  placeholder="# Title&#10;&#10;Write **bold**, *italic*, [links](https://...)&#10;&#10;- Bullet lists&#10;- `code`"
                  style={{ ...inputStyle, minHeight: 120, fontFamily: "var(--font-mono)", fontSize: 12, resize: 'vertical', marginTop: 6 }}
                />
              </div>
            )}
            <div style={{ gridColumn: '1 / -1', display: 'flex', alignItems: 'center', gap: 8, marginTop: 4 }}>
              <span style={{ ...labelTextStyle, minWidth: 55 }}>Shortcut</span>
              <div style={{ display: 'flex', alignItems: 'center', gap: 6, flex: 1 }}>
                {capturedShortcut ? (
                  <kbd style={{
                    fontFamily: "var(--font-mono)", fontSize: 12, fontWeight: 600,
                    padding: '4px 8px', borderRadius: 4,
                    background: 'var(--bg)', border: '1px solid var(--border)', color: 'var(--cyan)',
                  }}>{displayShortcut(capturedShortcut)}</kbd>
                ) : (
                  <span style={{ fontSize: 12, color: 'var(--muted)' }}>None</span>
                )}
                <button
                  onClick={() => setRecording(true)}
                  style={{
                    padding: '4px 10px', borderRadius: 4, fontSize: 11, cursor: 'pointer',
                    border: recording ? '1px solid var(--cyan)' : '1px solid var(--border)',
                    background: recording ? 'rgba(34,197,94,.1)' : 'var(--card)',
                    color: recording ? 'var(--cyan)' : 'var(--fg)',
                  }}
                >{recording ? 'Press a key...' : 'Record'}</button>
                {capturedShortcut && !recording && (
                  <button
                    onClick={() => setCapturedShortcut(null)}
                    style={{
                      padding: '4px 8px', borderRadius: 4, fontSize: 11, cursor: 'pointer',
                      border: '1px solid var(--crit)', background: 'transparent', color: 'var(--crit)',
                    }}
                  >Clear</button>
                )}
              </div>
            </div>
          </div>)}
          {tab === 'source' && (<div style={{ display: 'flex', flexDirection: 'column', gap: 8 }}>
            <label style={labelStyle}>
              <span style={labelTextStyle}>Data Source</span>
              <select value={form.source_type} onChange={e => { setForm({ ...form, source_type: e.target.value }); setSrcConfig({}); }} style={inputStyle}>
                <optgroup label="Real Data">
                  <option value="modbus">Modbus (Live Device)</option>
                </optgroup>
                <optgroup label="Simulate / Synthetic">
                  {(SIMULATE_TYPES as readonly string[]).map(t => <option key={t} value={t}>{SOURCE_CATEGORY[t]?.label ?? t}</option>)}
                </optgroup>
                <optgroup label="Other">
                  <option value="api">API (HTTP Poll)</option>
                  <option value="none">None (Static)</option>
                </optgroup>
              </select>
            </label>
            <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 8, marginTop: 4 }}><SourceConfigFields sourceType={form.source_type} config={srcConfig} onChange={c => {
              setSrcConfig(c);
              if (form.source_type === 'modbus' && c.modbus_device_id) {
                setForm(f => ({ ...f, device_id: c.modbus_device_id, key: c.modbus_register_name || f.key }));
              }
            }} /></div>
          </div>)}
        </div>
        </div>
        <div style={{ display: 'flex', gap: 8, padding: '0 16px 16px' }}>
          <button disabled={histErrors.length > 0} title={histErrors[0]} onClick={() => { onSave({ ...form, source_config: JSON.stringify(srcConfig) }, capturedShortcut); }} style={{ padding: '6px 16px', borderRadius: 6, border: '1px solid var(--cyan)', background: 'transparent', color: 'var(--cyan)', cursor: histErrors.length > 0 ? 'not-allowed' : 'pointer', opacity: histErrors.length > 0 ? 0.5 : 1, fontSize: 13 }}>Save</button>
          <button onClick={() => onDelete(sensor.id)} style={{ padding: '6px 16px', borderRadius: 6, border: '1px solid var(--crit)', background: 'transparent', color: 'var(--crit)', cursor: 'pointer', fontSize: 13 }}>Delete</button>
        </div>
      </div>
    </div>
  );
}
