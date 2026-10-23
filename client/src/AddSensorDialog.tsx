import { useState, useMemo } from 'react';
import { SensorConfig, SOURCE_CATEGORY, SIMULATE_TYPES } from './types';
import { inputStyle, monoInputStyle, labelStyle, labelTextStyle, tabStyle } from './styles';
import { SourceConfigFields } from './SourceConfigFields';
import { useNamingConvention, useAddSensor, useAddSensorToScreen } from './queries';

export function AddSensorDialog({ screenId, sensors, screenSensorIds, onClose, onDone }: {
  screenId: string; sensors: SensorConfig[]; screenSensorIds: number[];
  onClose: () => void; onDone: () => void;
}) {
  const [tab, setTab] = useState<'existing' | 'new'>('existing');
  const available = sensors.filter(s => !screenSensorIds.includes(s.id));
  const [newForm, setNewForm] = useState({ key: '', name: '', unit: '', min_val: 0, max_val: 100, warn_val: 80, crit_val: 90, base_val: 50, decimals: 1, source_type: 'random_walk', color: '#8b5cf6', device_id: '' });
  const [srcConfig, setSrcConfig] = useState<Record<string, any>>({});

  const { data: namingConvention } = useNamingConvention();
  const addSensorMut = useAddSensor()
  const addSensorToScreenMut = useAddSensorToScreen()

  const nameValidation = useMemo(() => {
    if (!namingConvention || !newForm.name) return null;
    const regex = namingConvention.custom_regex || namingConvention.regex;
    if (!regex) return null;
    try {
      const re = new RegExp(regex);
      return re.test(newForm.name);
    } catch {
      return null;
    }
  }, [namingConvention, newForm.name]);

  const addExisting = async (sensorId: number) => {
    await addSensorToScreenMut.mutateAsync({ screenId, sensorId });
    onDone();
  };

  const createNew = async () => {
    const payload = { ...newForm, source_config: JSON.stringify(srcConfig) };
    if (newForm.source_type === 'modbus' && srcConfig.modbus_device_id) {
      payload.device_id = srcConfig.modbus_device_id;
      payload.key = srcConfig.modbus_register_name || payload.key;
    }
    const { id } = await addSensorMut.mutateAsync(payload as any);
    await addSensorToScreenMut.mutateAsync({ screenId, sensorId: id });
    onDone();
  };

  const cat = SOURCE_CATEGORY[newForm.source_type];

  return (
    <div style={{ position: 'fixed', inset: 0, background: 'rgba(0,0,0,.6)', display: 'flex', alignItems: 'center', justifyContent: 'center', zIndex: 60 }} onClick={onClose}>
      <div style={{ background: 'var(--card)', border: '1px solid var(--border)', borderRadius: 10, width: 520, maxHeight: '85vh', display: 'flex', flexDirection: 'column' }} onClick={e => e.stopPropagation()}>
        <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', padding: '12px 16px', borderBottom: '1px solid var(--border)' }}>
          <div style={{ fontSize: 13, fontWeight: 600 }}>Add Data Source</div>
          <button onClick={onClose} style={{ background: 'none', border: 'none', color: 'var(--muted)', cursor: 'pointer', fontSize: 18 }}>&times;</button>
        </div>
        <div style={{ display: 'flex', borderBottom: '1px solid var(--border)' }}>
          <button style={tabStyle(tab === 'existing')} onClick={() => setTab('existing')}>Existing</button>
          <button style={tabStyle(tab === 'new')} onClick={() => setTab('new')}>Create New</button>
        </div>
        <div style={{ padding: 16, overflow: 'auto', flex: 1 }}>
          {tab === 'existing' ? (
            available.length === 0 ? <div style={{ color: 'var(--muted)', fontSize: 13, padding: 12 }}>All sources already on this screen.</div> :
            <div style={{ display: 'flex', flexDirection: 'column', gap: 4 }}>
              {available.map(s => {
                const sc = SOURCE_CATEGORY[s.source_type];
                return (
                  <button key={s.id} onClick={() => addExisting(s.id)} style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', padding: '8px 12px', background: 'var(--bg)', border: '1px solid var(--border)', borderRadius: 6, cursor: 'pointer', color: 'var(--fg)', width: '100%' }}>
                    <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
                      <div style={{ width: 8, height: 8, borderRadius: '50%', background: s.color }} />
                      <span style={{ fontWeight: 600, fontSize: 13 }}>{s.name}</span>
                      <span style={{ color: 'var(--muted)', fontSize: 11 }}>({s.key})</span>
                    </div>
                    <div style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
                      <span style={{ fontSize: 9, padding: '1px 6px', borderRadius: 3, background: `${sc?.color ?? 'var(--muted)'}18`, color: sc?.color ?? 'var(--muted)', fontWeight: 700, letterSpacing: '.03em' }}>{sc?.tag ?? s.source_type}</span>
                      <span style={{ fontSize: 11, color: 'var(--cyan)' }}>+ Add</span>
                    </div>
                  </button>
                );
              })}
            </div>
          ) : (
            <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 8 }}>
              <label style={labelStyle}><span style={labelTextStyle}>Key</span><input value={newForm.key} onChange={e => setNewForm({ ...newForm, key: e.target.value })} style={inputStyle} /></label>
              <label style={labelStyle}>
                <span style={labelTextStyle}>Name</span>
                <input
                  value={newForm.name}
                  onChange={e => setNewForm({ ...newForm, name: e.target.value })}
                  style={{
                    ...inputStyle,
                    borderColor: nameValidation === false ? 'var(--crit)' : nameValidation === true ? 'var(--green)' : 'var(--border)',
                  }}
                />
                {namingConvention && (
                  <div style={{ fontSize: 11, color: nameValidation === false ? 'var(--crit)' : 'var(--muted)', marginTop: 4 }}>
                    Pattern: <code style={{ fontFamily: "var(--font-mono)", background: 'var(--bg)', padding: '1px 4px', borderRadius: 3 }}>{namingConvention.custom_regex || namingConvention.regex}</code>
                    {nameValidation === false && <span style={{ marginLeft: 8, fontWeight: 600 }}>Does not match naming convention</span>}
                  </div>
                )}
              </label>
              <label style={{ ...labelStyle, gridColumn: '1 / -1' }}>
                <span style={labelTextStyle}>Data Source</span>
                <select value={newForm.source_type} onChange={e => { setNewForm({ ...newForm, source_type: e.target.value, device_id: '' }); setSrcConfig({}); }} style={inputStyle}>
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
              {cat && <div style={{ gridColumn: '1 / -1', display: 'flex', alignItems: 'center', gap: 6, marginBottom: 4 }}>
                <span style={{ fontSize: 10, padding: '2px 8px', borderRadius: 4, background: `${cat.color}18`, color: cat.color, fontWeight: 700, letterSpacing: '.04em' }}>{cat.tag}</span>
                <span style={{ fontSize: 11, color: 'var(--muted)' }}>{cat.label} {newForm.source_type === 'modbus' ? '— reads live data from device' : SIMULATE_TYPES.includes(newForm.source_type as any) ? '— generated synthetic data' : ''}</span>
              </div>}
              <div style={{ gridColumn: '1 / -1' }}>
                <SourceConfigFields sourceType={newForm.source_type} config={srcConfig} onChange={c => {
                  setSrcConfig(c);
                  if (newForm.source_type === 'modbus' && c.modbus_device_id) {
                    setNewForm(f => ({ ...f, device_id: c.modbus_device_id, key: c.modbus_register_name || f.key }));
                  }
                }} />
              </div>
              <label style={labelStyle}><span style={labelTextStyle}>Unit</span><input value={newForm.unit} onChange={e => setNewForm({ ...newForm, unit: e.target.value })} style={inputStyle} /></label>
              <label style={labelStyle}><span style={labelTextStyle}>Min</span><input type="number" step="any" value={newForm.min_val} onChange={e => setNewForm({ ...newForm, min_val: parseFloat(e.target.value) || 0 })} style={monoInputStyle} /></label>
              <label style={labelStyle}><span style={labelTextStyle}>Max</span><input type="number" step="any" value={newForm.max_val} onChange={e => setNewForm({ ...newForm, max_val: parseFloat(e.target.value) || 0 })} style={monoInputStyle} /></label>
              <label style={labelStyle}><span style={labelTextStyle}>Base Value</span><input type="number" step="any" value={newForm.base_val} onChange={e => setNewForm({ ...newForm, base_val: parseFloat(e.target.value) || 0 })} style={monoInputStyle} /></label>
              <button onClick={createNew} style={{ gridColumn: 'span 2', marginTop: 8, padding: '8px 16px', borderRadius: 6, border: '1px solid var(--cyan)', background: 'transparent', color: 'var(--cyan)', cursor: 'pointer', fontSize: 13, fontWeight: 600 }}>Create & Add</button>
            </div>
          )}
        </div>
      </div>
    </div>
  );
}
