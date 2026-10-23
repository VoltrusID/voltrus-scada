import { useState } from 'react';
import { inputStyle, monoInputStyle, labelStyle, labelTextStyle } from './styles';
import { useModbusDevices, useModbusRegisters } from './queries';

function ExpressionBuilder({ registers, value, onChange }: { registers: { name: string; address: number; register_type: string }[]; value: string; onChange: (v: string) => void }) {
  const parseTerms = (expr: string): string[] => {
    if (!expr) return [];
    const terms: string[] = [];
    let current = '';
    let inBrace = false;
    for (const ch of expr) {
      if (ch === '{') { inBrace = true; current += ch; }
      else if (ch === '}') { inBrace = false; current += ch; }
      else if ((ch === '+' || ch === '-' || ch === '*' || ch === '/') && !inBrace && current.trim()) {
        terms.push(current.trim());
        current = '';
      } else { current += ch; }
    }
    if (current.trim()) terms.push(current.trim());
    return terms;
  };

  const [terms, setTerms] = useState<string[]>(() => value ? parseTerms(value) : ['']);
  const [editingRaw, setEditingRaw] = useState(false);

  const buildExpression = (t: string[]) => {
    const ops = ['+', '-', '*', '/'];
    const parts: string[] = [];
    for (let i = 0; i < t.length; i++) {
      const part = t[i].trim();
      if (!part) continue;
      if (i === 0) {
        parts.push(part);
      } else if (ops.some(op => part.startsWith(op))) {
        parts.push(part);
      } else {
        parts.push('+ ' + part);
      }
    }
    return parts.join(' ');
  };

  const updateTerms = (newTerms: string[]) => {
    setTerms(newTerms);
    onChange(buildExpression(newTerms));
  };

  const addTerm = () => updateTerms([...terms, `+ {${registers[0]?.name ?? 'reg'}}`]);
  const removeTerm = (idx: number) => updateTerms(terms.filter((_, i) => i !== idx));

  const updateTerm = (idx: number, val: string) => {
    const next = [...terms];
    next[idx] = val;
    updateTerms(next);
  };

  const wrapRegister = (name: string) => `{${name}}`;

  const termRow = (term: string, idx: number) => {
    const isRegister = term.startsWith('{') && term.endsWith('}');
    const regName = isRegister ? term.slice(1, -1) : '';
    const ops = ['+', '-', '*', '/'];
    const leadingOp = ops.find(op => term.trimStart().startsWith(op));
    const needsOp = idx > 0 && !leadingOp;

    return (
      <div key={idx} style={{ display: 'flex', gap: 4, alignItems: 'center', marginBottom: 4 }}>
        {idx > 0 && (
          <select value={leadingOp ?? '+'} onChange={e => {
            const trimmed = term.trimStart().slice(leadingOp ? 1 : 0).trimStart();
            updateTerm(idx, `${e.target.value} ${trimmed}`);
          }} style={{ ...inputStyle, width: 48, padding: '4px 6px', fontSize: 12 }}>
            {ops.map(op => <option key={op} value={op}>{op}</option>)}
          </select>
        )}
        <select value={regName} onChange={e => {
          if (e.target.value) {
            updateTerm(idx, (needsOp ? '+ ' : '') + wrapRegister(e.target.value));
          }
        }} style={{ ...inputStyle, flex: 1, fontSize: 12 }}>
          <option value="">-- Register --</option>
          {registers.map(r => <option key={r.name} value={r.name}>{r.name}</option>)}
        </select>
        <span style={{ color: 'var(--muted)', fontSize: 11 }}>*&nbsp;</span>
        <input type="number" step="any" placeholder="1.0" value={
          (() => {
            if (!isRegister && !isNaN(Number(term))) return term;
            return '';
          })()
        } onChange={e => {
          if (e.target.value) {
            updateTerm(idx, (needsOp ? '+ ' : '') + e.target.value);
          }
        }} style={{ ...monoInputStyle, width: 80, fontSize: 12 }} />
        <button type="button" onClick={() => removeTerm(idx)} style={{ padding: '2px 8px', border: '1px solid var(--crit)', background: 'transparent', color: 'var(--crit)', borderRadius: 3, cursor: 'pointer', fontSize: 11 }}>x</button>
      </div>
    );
  };

  return (
    <div style={{ border: '1px solid var(--border)', borderRadius: 6, padding: 8 }}>
      <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', marginBottom: 6 }}>
        <span style={{ fontSize: 11, fontWeight: 600, color: 'var(--muted)', textTransform: 'uppercase', letterSpacing: '.05em' }}>Expression</span>
        <button type="button" onClick={() => setEditingRaw(!editingRaw)} style={{ fontSize: 10, padding: '2px 8px', border: '1px solid var(--border)', background: 'transparent', color: 'var(--muted)', borderRadius: 3, cursor: 'pointer' }}>
          {editingRaw ? 'Builder' : 'Raw'}
        </button>
      </div>

      {editingRaw ? (
        <input type="text" value={value} onChange={e => { onChange(e.target.value); setTerms(parseTerms(e.target.value)); }} placeholder="{Reg_A} + {Reg_B} * 0.5" style={{ ...inputStyle, fontFamily: "var(--font-mono)", fontSize: 12, width: '100%' }} />
      ) : (
        <>
          {terms.map(termRow)}
          <button type="button" onClick={addTerm} style={{ marginTop: 4, padding: '4px 12px', borderRadius: 4, border: '1px solid var(--border)', background: 'transparent', color: 'var(--cyan)', cursor: 'pointer', fontSize: 11 }}>+ Add Term</button>
        </>
      )}

      {value && (
        <div style={{ marginTop: 6, padding: '4px 8px', background: 'rgba(34,197,94,.05)', borderRadius: 4, fontFamily: "var(--font-mono)", fontSize: 11, color: 'var(--fg)' }}>
          = {value}
        </div>
      )}
    </div>
  );
}

export function SourceConfigFields({ sourceType, config, onChange }: { sourceType: string; config: Record<string, any>; onChange: (c: Record<string, any>) => void }) {
  const set = (k: string, v: any) => onChange({ ...config, [k]: v });
  const { data: modbusDevices = [] } = useModbusDevices(sourceType === 'modbus');
  const { data: registers = [] } = useModbusRegisters(config.modbus_device_id ?? '', sourceType === 'modbus' && !!config.modbus_device_id);
  switch (sourceType) {
    case 'random_walk': return (<>
      <label style={labelStyle}><span style={labelTextStyle}>Volatility</span><input type="range" min="0.01" max="0.5" step="0.01" value={config.volatility ?? 0.06} onChange={e => set('volatility', parseFloat(e.target.value))} style={{ width: '100%' }} /><span style={{ fontSize: 11, color: 'var(--fg)', fontFamily: 'var(--font-mono)' }}>{(config.volatility ?? 0.06).toFixed(2)}</span></label>
      <label style={labelStyle}><span style={labelTextStyle}>Reversion</span><input type="range" min="0.01" max="0.2" step="0.01" value={config.reversion ?? 0.03} onChange={e => set('reversion', parseFloat(e.target.value))} style={{ width: '100%' }} /><span style={{ fontSize: 11, color: 'var(--fg)', fontFamily: 'var(--font-mono)' }}>{(config.reversion ?? 0.03).toFixed(2)}</span></label></>);
    case 'sine': return (<>
      <label style={labelStyle}><span style={labelTextStyle}>Amplitude</span><input type="number" step="any" value={config.amplitude ?? 5} onChange={e => set('amplitude', parseFloat(e.target.value))} style={monoInputStyle} /></label>
      <label style={labelStyle}><span style={labelTextStyle}>Frequency (Hz)</span><input type="number" step="any" value={config.frequency ?? 0.5} onChange={e => set('frequency', parseFloat(e.target.value))} style={monoInputStyle} /></label>
      <label style={labelStyle}><span style={labelTextStyle}>Phase Offset</span><input type="number" step="any" value={config.offset ?? 0} onChange={e => set('offset', parseFloat(e.target.value))} style={monoInputStyle} /></label></>);
    case 'step': return (<>
      <label style={labelStyle}><span style={labelTextStyle}>Jump Probability</span><input type="number" step="0.01" min="0" max="1" value={config.jump_probability ?? 0.02} onChange={e => set('jump_probability', parseFloat(e.target.value))} style={monoInputStyle} /></label>
      <label style={labelStyle}><span style={labelTextStyle}>Settle Time (s)</span><input type="number" step="1" min="1" value={config.settle_time ?? 5} onChange={e => set('settle_time', parseFloat(e.target.value))} style={monoInputStyle} /></label></>);
    case 'sawtooth': return <label style={labelStyle}><span style={labelTextStyle}>Period (s)</span><input type="number" step="any" min="1" value={config.period ?? 10} onChange={e => set('period', parseFloat(e.target.value))} style={monoInputStyle} /></label>;
    case 'constant': return <div style={{ color: 'var(--muted)', fontSize: 12, padding: 8 }}>Always returns base value.</div>;
    case 'none': return <div style={{ color: 'var(--muted)', fontSize: 12, padding: 8 }}>No data source. Use for image cards or static displays.</div>;
    case 'noise': return <label style={labelStyle}><span style={labelTextStyle}>Noise Amplitude</span><input type="number" step="any" value={config.noise_amplitude ?? 3} onChange={e => set('noise_amplitude', parseFloat(e.target.value))} style={monoInputStyle} /></label>;
    case 'api': return (<>
      <label style={labelStyle}><span style={labelTextStyle}>URL</span><input type="text" value={config.url ?? ''} onChange={e => set('url', e.target.value)} style={inputStyle} placeholder="https://example.com/api/v1/sensor" /></label>
      <label style={labelStyle}><span style={labelTextStyle}>JSON Path</span><input type="text" value={config.path ?? ''} onChange={e => set('path', e.target.value)} style={inputStyle} placeholder="data.value" /></label>
      <label style={labelStyle}><span style={labelTextStyle}>Method</span><select value={config.method ?? 'GET'} onChange={e => set('method', e.target.value)} style={inputStyle}><option value="GET">GET</option><option value="POST">POST</option></select></label>
      <label style={labelStyle}><span style={labelTextStyle}>Poll Interval (ms)</span><input type="number" step="1000" min="1000" value={config.poll_interval ?? 5000} onChange={e => set('poll_interval', parseInt(e.target.value))} style={monoInputStyle} /></label>
      <label style={labelStyle}><span style={labelTextStyle}>Headers (JSON)</span><textarea value={typeof config.headers === 'object' ? JSON.stringify(config.headers, null, 2) : config.headers ?? '{}'} onChange={e => { try { set('headers', JSON.parse(e.target.value)) } catch { set('headers', e.target.value) } }} style={{ ...inputStyle, minHeight: 60, fontFamily: 'var(--font-mono)', fontSize: 11 }} /></label></>);
    case 'modbus': {
      const modbusMode = config.modbus_mode ?? (config.expression ? 'expression' : 'single');
      const setMode = (mode: string) => onChange({ ...config, modbus_mode: mode, expression: mode === 'single' ? '' : (config.expression ?? ''), modbus_register_name: mode === 'expression' ? '' : (config.modbus_register_name ?? '') });

      return (<div style={{ gridColumn: '1 / -1' }}>
        <label style={labelStyle}>
          <span style={labelTextStyle}>Modbus Device</span>
          <select value={config.modbus_device_id ?? ''} onChange={e => onChange({ ...config, modbus_device_id: e.target.value, modbus_register_name: '', expression: '' })} style={inputStyle}>
            <option value="">-- Select Device --</option>
            {modbusDevices.map(d => <option key={d.id} value={d.id}>{d.name || d.id} ({d.ip}:{d.port})</option>)}
          </select>
        </label>
        {config.modbus_device_id && (
          <div style={{ marginTop: 8 }}>
            <div style={{ display: 'flex', gap: 4, marginBottom: 8 }}>
              <button type="button" onClick={() => setMode('single')} style={{ padding: '4px 12px', borderRadius: 4, border: `1px solid ${modbusMode === 'single' ? 'var(--cyan)' : 'var(--border)'}`, background: modbusMode === 'single' ? 'rgba(34,197,94,.1)' : 'transparent', color: modbusMode === 'single' ? 'var(--cyan)' : 'var(--muted)', cursor: 'pointer', fontSize: 11 }}>Single Register</button>
              <button type="button" onClick={() => setMode('expression')} style={{ padding: '4px 12px', borderRadius: 4, border: `1px solid ${modbusMode === 'expression' ? 'var(--cyan)' : 'var(--border)'}`, background: modbusMode === 'expression' ? 'rgba(34,197,94,.1)' : 'transparent', color: modbusMode === 'expression' ? 'var(--cyan)' : 'var(--muted)', cursor: 'pointer', fontSize: 11 }}>Expression</button>
            </div>

            {modbusMode === 'single' ? (
              <label style={labelStyle}>
                <span style={labelTextStyle}>Register</span>
                <select value={config.modbus_register_name ?? ''} onChange={e => set('modbus_register_name', e.target.value)} style={inputStyle}>
                  <option value="">-- Select Register --</option>
                  {registers.map(r => <option key={r.name} value={r.name}>{r.name} (addr {r.address}, {r.register_type})</option>)}
                </select>
              </label>
            ) : (
              <ExpressionBuilder registers={registers} value={config.expression ?? ''} onChange={expr => set('expression', expr)} />
            )}
          </div>
        )}
        {!config.modbus_device_id && <div style={{ color: 'var(--muted)', fontSize: 12, padding: 8 }}>Select a Modbus device to read live data from its registers.</div>}
      </div>);
    }
    default: return null;
  }
}
