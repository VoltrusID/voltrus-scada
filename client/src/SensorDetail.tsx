import { useEffect, useState, useRef, useMemo, useCallback } from 'react';
import { SensorConfig } from './types';
import { stColor, btnStyle, safeMinMax, displayShortcut, inputStyle, labelTextStyle } from './styles';
import { TvDetailChart, OverlaySeries, AnnotationMarker } from './TvDetailChart';
import { HistoryRangeBar } from './CustomRangePicker';
import { useDetailData, useSensorCurrentValue, useDashboardSelector } from './store/dashboardStore';
import { WriteDialog } from './components/WriteDialog';
import { WritePolicyEditor } from './components/WritePolicyEditor';
import { useAnnotations, useCreateAnnotation, useDeleteAnnotation } from './queries/annotations';
import { useQuery } from '@tanstack/react-query';
import { getWritePolicy } from './api/commands';

export function SensorDetail({ sensor, onBack, historyRange, onRangeChange, historyDetail, onDetailChange, historyAutoAgg, shortcuts, mode, onModeChange, allSensors, isAdmin }: {
  sensor: SensorConfig; onBack: () => void; historyRange: string; onRangeChange: (r: string) => void;
  historyDetail?: 'auto' | 'full'; onDetailChange?: (d: 'auto' | 'full') => void;
  historyAutoAgg?: string;
  shortcuts: Record<number, string>;
  mode: 'live' | 'history' | 'flows'; onModeChange: (m: 'live' | 'history' | 'flows') => void;
  allSensors?: SensorConfig[];
  isAdmin?: boolean;
}) {
  const data = useDetailData(sensor.id, mode)
  const currentValue = useSensorCurrentValue(sensor.id)
  const allCurrentValues = useDashboardSelector(s => s.currentValues)
  const dataMap = useDashboardSelector(s => s.dataMap)

  const st = currentValue?.status ?? 'normal';
  const isOffline = st === 'offline';
  const curV = isOffline ? null : currentValue ? currentValue.value : sensor.base_val;

  const [overlayIds, setOverlayIds] = useState<number[]>([]);
  const [showPicker, setShowPicker] = useState(false);
  const pickerRef = useRef<HTMLDivElement>(null);
  const [showWriteDialog, setShowWriteDialog] = useState(false);
  // The stored policy (admin only: the write controls are); the dialogs open with it, not with defaults.
  const { data: writePolicy, refetch: refetchWritePolicy } = useQuery({
    queryKey: ['write-policy', sensor.id],
    queryFn: () => getWritePolicy(sensor.id),
    enabled: !!isAdmin,
    staleTime: 0,
  });
  const [showPolicyEditor, setShowPolicyEditor] = useState(false);

  // ── Annotation state ──
  const [annotationTime, setAnnotationTime] = useState<number | null>(null)
  const [annotationText, setAnnotationText] = useState('')
  const [annotationPopoverPos, setAnnotationPopoverPos] = useState<{ x: number; y: number } | null>(null)
  const [showAnnotationList, setShowAnnotationList] = useState(false)

  const { data: annotationsData } = useAnnotations({ tag_id: sensor.key })
  const annotations = annotationsData?.annotations ?? []
  const createAnnotationMut = useCreateAnnotation()
  const deleteAnnotationMut = useDeleteAnnotation()

  const chartAnnotations: AnnotationMarker[] = useMemo(() =>
    annotations.map(a => ({ id: a.id, time: a.data_timestamp, text: a.annotation_text })),
    [annotations]
  )

  const handleAnnotationClick = useCallback((time: number) => {
    setAnnotationTime(time)
    setAnnotationText('')
  }, [])

  const handleCreateAnnotation = () => {
    if (!annotationText.trim() || annotationTime === null) return
    createAnnotationMut.mutate(
      { tag_id: sensor.key, annotation_text: annotationText.trim(), data_timestamp: Math.floor(annotationTime) },
      { onSuccess: () => { setAnnotationTime(null); setAnnotationText('') } }
    )
  }

  const handleDeleteAnnotation = (id: number) => {
    deleteAnnotationMut.mutate(id)
  }

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => { if (e.key === 'Escape') onBack(); };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [onBack]);

  useEffect(() => {
    const onClick = (e: MouseEvent) => {
      if (showPicker && pickerRef.current && !pickerRef.current.contains(e.target as Node)) setShowPicker(false);
    };
    document.addEventListener('mousedown', onClick);
    return () => document.removeEventListener('mousedown', onClick);
  }, [showPicker]);

  const stats = useMemo(() => {
    const safeValues = data.map(d => d.value).filter(v => v != null && !Number.isNaN(v));
    const [minV, maxV] = safeMinMax(safeValues);
    const avgV = safeValues.length ? safeValues.reduce((a, b) => a + b, 0) / safeValues.length : 0;
    return { safeValues, minV, maxV, avgV, pointCount: data.length };
  }, [data]);

  const overlays: OverlaySeries[] = useMemo(() => {
    if (!allSensors || overlayIds.length === 0) return [];
    return overlayIds.map(id => {
      const s = allSensors.find(x => x.id === id);
      if (!s) return null;
      const pts = dataMap[String(s.id)] ?? [];
      return { data: pts, color: s.color, name: s.name, unit: s.unit };
    }).filter(Boolean) as OverlaySeries[];
  }, [allSensors, overlayIds, dataMap]);

  const sparklineSensors = useMemo(() =>
    (allSensors ?? []).filter(s => s.chart_type === 'sparkline' && s.id !== sensor.id && !overlayIds.includes(s.id)),
    [allSensors, sensor.id, overlayIds]
  );

  const toggleOverlay = (id: number) => {
    setOverlayIds(prev => prev.includes(id) ? prev.filter(x => x !== id) : [...prev, id]);
    setShowPicker(false);
  };

  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 8, height: '100%', overflow: 'hidden' }}>
      {/* Header row */}
      <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', flexShrink: 0 }}>
        <div style={{ display: 'flex', alignItems: 'center', gap: 10 }}>
          <button onClick={onBack} style={{ ...btnStyle, display: 'flex', alignItems: 'center', gap: 4, padding: '4px 8px' }}>
            <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2"><path d="M19 12H5M12 19l-7-7 7-7" /></svg>Back<kbd style={{ fontFamily: "var(--font-mono)", fontSize: 8, padding: '1px 3px', marginLeft: 2, borderRadius: 3, background: 'var(--bg)', border: '1px solid var(--border)', color: 'var(--muted)' }}>Esc</kbd>
          </button>
          <h1 style={{ fontSize: 16, fontWeight: 700 }}>{sensor.name}</h1>
          {shortcuts[sensor.id] && (
            <kbd style={{ fontFamily: "var(--font-mono)", fontSize: 10, fontWeight: 600, padding: '1px 6px', borderRadius: 3, background: 'var(--bg)', border: '1px solid var(--border)', color: 'var(--muted)' }}>{displayShortcut(shortcuts[sensor.id])}</kbd>
          )}
          <span style={{ fontSize: 11, color: 'var(--muted)' }}>({sensor.key})</span>
        </div>
        <div style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
          <a href={`/api/v1/export/csv?sensor=${sensor.key}&range=${historyRange}`} download style={{ ...btnStyle, textDecoration: 'none', padding: '3px 8px', fontSize: 11 }}>
            <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2"><path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4"/><polyline points="7 10 12 15 17 10"/><line x1="12" y1="15" x2="12" y2="3"/></svg>CSV
          </a>
          <div style={{ padding: '2px 8px', borderRadius: 4, fontWeight: 600, fontSize: 10, textTransform: 'uppercase', background: st === 'critical' ? 'rgba(239,68,68,.15)' : st === 'warning' ? 'rgba(234,179,8,.15)' : st === 'offline' ? 'rgba(100,100,100,.15)' : 'rgba(34,197,94,.15)', color: stColor(st) }}>{st}</div>
          {isAdmin && (
            <button
              onClick={() => setShowWriteDialog(true)}
              disabled={!writePolicy}
              style={{ ...btnStyle, padding: '3px 8px', fontSize: 11, borderColor: 'var(--cyan)', color: 'var(--cyan)' }}
              aria-label="Write value to tag"
            >
              <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2"><path d="M12 20h9"/><path d="M16.5 3.5a2.121 2.121 0 0 1 3 3L7 19l-4 1 1-4L16.5 3.5z"/></svg>
              Write
            </button>
          )}
          {isAdmin && (
            <button
              onClick={() => setShowPolicyEditor(true)}
              disabled={!writePolicy}
              style={{ ...btnStyle, padding: '3px 8px', fontSize: 11, borderColor: 'var(--border)', color: 'var(--muted)' }}
              aria-label="Toggle write policy editor"
            >
              <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2"><path d="M12.22 2h-.44a2 2 0 0 0-2 2v.18a2 2 0 0 1-1 1.73l-.43.25a2 2 0 0 1-2 0l-.15-.08a2 2 0 0 0-2.73.73l-.22.38a2 2 0 0 0 .73 2.73l.15.1a2 2 0 0 1 1 1.72v.51a2 2 0 0 1-1 1.74l-.15.09a2 2 0 0 0-.73 2.73l.22.38a2 2 0 0 0 2.73.73l.15-.08a2 2 0 0 1 2 0l.43.25a2 2 0 0 1 1 1.73V20a2 2 0 0 0 2 2h.44a2 2 0 0 0 2-2v-.18a2 2 0 0 1 1-1.73l.43-.25a2 2 0 0 1 2 0l.15.08a2 2 0 0 0 2.73-.73l.22-.39a2 2 0 0 0-.73-2.73l-.15-.08a2 2 0 0 1-1-1.74v-.5a2 2 0 0 1 1-1.74l.15-.09a2 2 0 0 0 .73-2.73l-.22-.38a2 2 0 0 0-2.73-.73l-.15.08a2 2 0 0 1-2 0l-.43-.25a2 2 0 0 1-1-1.73V4a2 2 0 0 0-2-2z"/><circle cx="12" cy="12" r="3"/></svg>
              Policy
            </button>
          )}
        </div>
      </div>

      {/* Compact stats row */}
      <div style={{ display: 'grid', gridTemplateColumns: 'auto repeat(4, 1fr) auto', gap: 6, flexShrink: 0 }}>
        <div style={{ background: 'var(--card)', border: '1px solid var(--border)', borderRadius: 6, padding: '6px 12px', display: 'flex', alignItems: 'center', gap: 6 }}>
          <div style={{ fontSize: 10, color: 'var(--muted)', textTransform: 'uppercase', letterSpacing: '.05em' }}>Current</div>
          <div style={{ fontFamily: "var(--font-mono)", fontSize: 18, fontWeight: 700, color: st === 'normal' ? sensor.color : stColor(st) }}>
            {curV != null ? curV.toFixed(sensor.decimals) : 'OFFLINE'}<span style={{ fontSize: 11, color: 'var(--muted)', fontWeight: 400, marginLeft: 3 }}>{sensor.unit}</span>
          </div>
        </div>
        {[{ l: 'Min', v: stats.safeValues.length ? stats.minV.toFixed(sensor.decimals) : '—' }, { l: 'Max', v: stats.safeValues.length ? stats.maxV.toFixed(sensor.decimals) : '—' }, { l: 'Avg', v: stats.safeValues.length ? stats.avgV.toFixed(sensor.decimals) : '—' }, { l: 'Pts', v: String(stats.pointCount) }].map((s, i) => (
          <div key={i} style={{ background: 'var(--card)', border: '1px solid var(--border)', borderRadius: 6, padding: '6px 10px' }}>
            <div style={{ fontSize: 9, color: 'var(--muted)', textTransform: 'uppercase', letterSpacing: '.05em' }}>{s.l}</div>
            <div style={{ fontFamily: "var(--font-mono)", fontSize: 13, fontWeight: 700, marginTop: 1 }}>{s.v}</div>
          </div>
        ))}
        <div style={{ background: 'var(--card)', border: '1px solid var(--border)', borderRadius: 6, padding: '6px 10px', display: 'flex', flexDirection: 'column', justifyContent: 'center', gap: 1 }}>
          {[
            { l: 'Warn', v: sensor.warn_val, c: 'var(--warn)' },
            { l: 'Crit', v: sensor.crit_val, c: 'var(--crit)' },
            ...((sensor.on_delay_secs > 0 || sensor.off_delay_secs > 0) ? [
              { l: 'On delay', v: `${sensor.on_delay_secs}s`, c: 'var(--muted)' },
              { l: 'Off delay', v: `${sensor.off_delay_secs}s`, c: 'var(--muted)' },
            ] : []),
          ].map((t, i) => (
            <div key={i} style={{ display: 'flex', justifyContent: 'space-between', gap: 8, fontSize: 10 }}>
              <span style={{ color: 'var(--muted)' }}>{t.l}</span>
              <span style={{ fontFamily: "var(--font-mono)", color: t.c }}>{t.v}{sensor.unit ? sensor.unit : ''}</span>
            </div>
          ))}
        </div>
      </div>

      {/* Chart area */}
      <div style={{ background: 'var(--card)', border: '1px solid var(--border)', borderRadius: 8, padding: 8, flex: 1, minHeight: 0, overflow: 'hidden', display: 'flex', flexDirection: 'column' }}>
        {/* Overlay bar */}
        <div style={{ display: 'flex', alignItems: 'center', gap: 5, marginBottom: 6, flexShrink: 0, flexWrap: 'wrap' }}>
          <span style={{ fontSize: 10, color: 'var(--muted)', fontWeight: 500, textTransform: 'uppercase', letterSpacing: '.05em' }}>Overlays:</span>
          {overlayIds.map(id => {
            const s = (allSensors ?? []).find(x => x.id === id);
            if (!s) return null;
            return (
              <div key={id} style={{ display: 'flex', alignItems: 'center', gap: 3, padding: '1px 6px', borderRadius: 3, background: s.color + '20', border: `1px solid ${s.color}40`, fontSize: 10, fontFamily: "var(--font-mono)", color: s.color, cursor: 'pointer' }} onClick={() => setOverlayIds(prev => prev.filter(x => x !== id))}>
                <div style={{ width: 6, height: 2, background: s.color, borderRadius: 1 }} />
                {s.name}
                <span style={{ opacity: 0.6, marginLeft: 1 }}>x</span>
              </div>
            );
          })}
          <div ref={pickerRef} style={{ position: 'relative' }}>
            <button onClick={() => setShowPicker(!showPicker)} style={{ ...btnStyle, fontSize: 10, padding: '2px 8px', borderColor: sparklineSensors.length ? 'var(--cyan)' : 'var(--border)', color: sparklineSensors.length ? 'var(--cyan)' : 'var(--muted)', opacity: sparklineSensors.length ? 1 : 0.5 }}>
              + Add
            </button>
            {showPicker && sparklineSensors.length > 0 && (
              <div style={{ position: 'absolute', top: '100%', left: 0, marginTop: 4, background: 'var(--card)', border: '1px solid var(--border)', borderRadius: 6, padding: 5, minWidth: 200, maxHeight: 200, overflowY: 'auto', zIndex: 10, boxShadow: '0 8px 24px rgba(0,0,0,.4)' }}>
                <div style={{ fontSize: 9, color: 'var(--muted)', padding: '2px 5px 5px', textTransform: 'uppercase', letterSpacing: '.05em' }}>Sparkline sensors</div>
                {sparklineSensors.map(s => {
                  const cv = allCurrentValues[String(s.id)]
                  const curVal = cv ? cv.value : null
                  return (
                    <button key={s.id} onClick={() => toggleOverlay(s.id)} style={{ display: 'flex', alignItems: 'center', gap: 6, width: '100%', padding: '5px 6px', border: 'none', background: 'none', borderRadius: 3, cursor: 'pointer', color: 'var(--fg)', fontSize: 11, textAlign: 'left' }}
                      onMouseEnter={e => (e.currentTarget.style.background = 'rgba(255,255,255,.05)')}
                      onMouseLeave={e => (e.currentTarget.style.background = 'none')}>
                      <div style={{ width: 8, height: 8, borderRadius: 2, background: s.color, flexShrink: 0 }} />
                      <div style={{ flex: 1, minWidth: 0 }}>
                        <div style={{ fontWeight: 600, whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis' }}>{s.name}</div>
                        <div style={{ fontSize: 9, color: 'var(--muted)' }}>{s.key}</div>
                      </div>
                      {curVal != null && <div style={{ fontFamily: "var(--font-mono)", fontSize: 10, color: s.color, fontWeight: 600 }}>{curVal.toFixed(s.decimals)}</div>}
                    </button>
                  );
                })}
              </div>
            )}
          </div>
        </div>
        <div style={{ flex: 1, minHeight: 0, position: 'relative' }}>
          <TvDetailChart data={data} color={sensor.color} warn={sensor.warn_val} crit={sensor.crit_val} unit={sensor.unit} overlays={overlays} annotations={chartAnnotations} onAnnotationClick={handleAnnotationClick} />

          {/* Top-left toolbar: hint + notes toggle */}
          <div style={{ position: 'absolute', top: 8, left: 8, display: 'flex', alignItems: 'center', gap: 6, zIndex: 5 }}>
            <div style={{
              fontSize: 10, color: 'var(--muted)',
              background: 'var(--card)', border: '1px solid var(--border)',
              borderRadius: 4, padding: '2px 8px', opacity: 0.7,
            }}>
              Right-click to annotate
            </div>
            {annotations.length > 0 && (
              <button
                onClick={() => setShowAnnotationList(v => !v)}
                style={{
                  ...btnStyle, fontSize: 11, padding: '4px 10px',
                  borderColor: showAnnotationList ? '#60a5fa' : 'var(--border)',
                  color: showAnnotationList ? '#60a5fa' : 'var(--muted)',
                  background: 'var(--card)',
                }}
              >
                <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2"><path d="M20.59 13.41l-7.17 7.17a2 2 0 0 1-2.83 0L2 12V2h10l8.59 8.59a2 2 0 0 1 0 2.82z"/><line x1="7" y1="7" x2="7.01" y2="7"/></svg>
                {annotations.length} note{annotations.length !== 1 ? 's' : ''}
              </button>
            )}
          </div>

          {/* Annotation list panel */}
          {showAnnotationList && annotations.length > 0 && (
            <div style={{
              position: 'absolute', top: 40, left: 8, width: 320, maxHeight: 280,
              background: 'var(--card)', border: '1px solid var(--border)',
              borderRadius: 8, overflow: 'hidden', zIndex: 5,
              boxShadow: '0 8px 24px rgba(0,0,0,0.3)', display: 'flex', flexDirection: 'column',
            }}>
              <div style={{
                padding: '10px 14px', borderBottom: '1px solid var(--border)',
                display: 'flex', alignItems: 'center', justifyContent: 'space-between', flexShrink: 0,
              }}>
                <span style={{ fontSize: 13, fontWeight: 600 }}>Annotations</span>
                <button
                  onClick={() => setShowAnnotationList(false)}
                  style={{ background: 'none', border: 'none', color: 'var(--muted)', cursor: 'pointer', fontSize: 16, lineHeight: 1, padding: 2 }}
                >&times;</button>
              </div>
              <div style={{ overflowY: 'auto', flex: 1 }}>
                {annotations.map(ann => (
                  <div key={ann.id} style={{
                    padding: '8px 14px', borderBottom: '1px solid var(--border)',
                    fontSize: 12, display: 'flex', gap: 8, alignItems: 'flex-start',
                    transition: 'background 0.1s',
                  }}
                    onMouseEnter={e => { (e.currentTarget as HTMLElement).style.background = 'var(--hover-overlay)' }}
                    onMouseLeave={e => { (e.currentTarget as HTMLElement).style.background = 'transparent' }}
                  >
                    <div style={{
                      width: 6, height: 6, borderRadius: '50%', background: '#60a5fa',
                      flexShrink: 0, marginTop: 5,
                    }} />
                    <div style={{ flex: 1, minWidth: 0 }}>
                      <div style={{ fontSize: 10, color: 'var(--muted)', fontFamily: "var(--font-mono)" }}>
                        {new Date(ann.data_timestamp * 1000).toLocaleString()}
                      </div>
                      <div style={{ marginTop: 2, lineHeight: 1.4 }}>{ann.annotation_text}</div>
                    </div>
                    <button
                      onClick={() => handleDeleteAnnotation(ann.id)}
                      style={{ background: 'none', border: 'none', color: 'var(--crit)', cursor: 'pointer', fontSize: 11, padding: 2, opacity: 0.5, flexShrink: 0 }}
                      title="Delete annotation"
                    >
                      <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2"><polyline points="3 6 5 6 21 6"/><path d="M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6m3 0V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2"/></svg>
                    </button>
                  </div>
                ))}
              </div>
            </div>
          )}

          {/* Right-click annotation popover */}
          {annotationTime !== null && (
            <div style={{
              position: 'absolute', top: 12, right: 12, width: 280,
              background: 'var(--card)', border: '1px solid var(--border)',
              borderRadius: 8, padding: 14, zIndex: 10,
              boxShadow: '0 8px 24px rgba(0,0,0,0.4)',
            }}>
              <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', marginBottom: 8 }}>
                <div style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
                  <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="var(--cyan)" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round"><path d="M20.59 13.41l-7.17 7.17a2 2 0 0 1-2.83 0L2 12V2h10l8.59 8.59a2 2 0 0 1 0 2.82z"/><line x1="7" y1="7" x2="7.01" y2="7"/></svg>
                  <span style={{ fontSize: 12, fontWeight: 600 }}>Add Note</span>
                </div>
                <button onClick={() => setAnnotationTime(null)} style={{ background: 'none', border: 'none', color: 'var(--muted)', cursor: 'pointer', fontSize: 16, lineHeight: 1, padding: 2 }}>&times;</button>
              </div>
              <div style={{ fontSize: 11, color: 'var(--muted)', fontFamily: "var(--font-mono)", marginBottom: 8 }}>
                {new Date(annotationTime * 1000).toLocaleString()}
              </div>
              <textarea
                autoFocus
                value={annotationText}
                onChange={e => setAnnotationText(e.target.value)}
                placeholder="Annotation text..."
                rows={2}
                style={{ ...inputStyle, resize: 'vertical', minHeight: 48, marginBottom: 8 }}
                onKeyDown={e => { if (e.key === 'Enter' && (e.metaKey || e.ctrlKey)) handleCreateAnnotation() }}
              />
              <button
                onClick={handleCreateAnnotation}
                disabled={!annotationText.trim() || createAnnotationMut.isPending}
                style={{
                  ...btnStyle, width: '100%', justifyContent: 'center',
                  borderColor: 'var(--cyan)', color: 'var(--cyan)',
                  opacity: (!annotationText.trim() || createAnnotationMut.isPending) ? 0.5 : 1,
                }}
              >
                {createAnnotationMut.isPending ? 'Saving...' : 'Save Annotation'}
                <span style={{ fontSize: 10, color: 'var(--muted)', marginLeft: 4 }}>⌘+Enter</span>
              </button>
              {createAnnotationMut.isError && (
                <div style={{ fontSize: 11, color: 'var(--crit)', marginTop: 6 }}>{createAnnotationMut.error?.message || 'Failed'}</div>
              )}
            </div>
          )}
        </div>
      </div>

      {/* Write Policy dialog */}
      {showPolicyEditor && isAdmin && writePolicy && (
        <WritePolicyEditor
          tagId={sensor.id}
          tagName={sensor.name}
          currentPolicy={writePolicy}
          isAdmin={!!isAdmin}
          onSave={() => { refetchWritePolicy() }}
          onClose={() => setShowPolicyEditor(false)}
        />
      )}

      {/* Write dialog */}
      {showWriteDialog && writePolicy && (
        <WriteDialog
          tagId={sensor.id}
          tagName={sensor.name}
          currentValue={curV}
          policy={writePolicy}
          onClose={() => setShowWriteDialog(false)}
        />
      )}
    </div>
  );
}
