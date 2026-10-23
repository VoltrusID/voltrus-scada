import { useRef, useEffect, useMemo } from 'react';
import { marked } from 'marked';
import DOMPurify from 'dompurify';
import { safeMinMax, toHexColor } from './styles';
import { useCanvasSize } from './hooks/useCanvasSize';

// ── Sparkline ──

function SparklineChart({ data, color: rawColor, warn, crit, simulated }: { data: number[]; color: string; warn: number; crit: number; simulated?: boolean }) {
  const color = toHexColor(rawColor.replace(/"/g, ''));
  const containerRef = useRef<HTMLDivElement>(null)
  const canvasRef = useRef<HTMLCanvasElement>(null)
  const size = useCanvasSize(containerRef)

  useEffect(() => {
    const cv = canvasRef.current;
    if (!cv || data.length < 2) return;
    const ctx = cv.getContext('2d')!
    const { width: W, height: H, dpr } = size
    if (W <= 0 || H <= 0) return

    cv.width = W * dpr; cv.height = H * dpr; ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    const pL = 40, pR = 10, pT = 10, pB = 24, cW = W - pL - pR, cH = H - pT - pB;
    ctx.clearRect(0, 0, W, H);
    ctx.strokeStyle = '#1a1a1e'; ctx.lineWidth = 1;
    for (let i = 0; i <= 4; i++) { const y = pT + (cH / 4) * i; ctx.beginPath(); ctx.moveTo(pL, y); ctx.lineTo(W - pR, y); ctx.stroke(); }
    const [rawMin, rawMax] = safeMinMax(data);
    let yMin = rawMin - 2, yMax = rawMax + 2; if (yMax - yMin < 4) { yMin--; yMax++; }
    const toX = (i: number) => pL + (i / (data.length - 1)) * cW;
    const toY = (v: number) => pT + cH - ((v - yMin) / (yMax - yMin)) * cH;
    ctx.fillStyle = '#52525b'; ctx.font = "10px 'SF Mono','JetBrains Mono',Menlo,monospace"; ctx.textAlign = 'right';
    for (let i = 0; i <= 4; i++) { const v = yMin + ((yMax - yMin) / 4) * (4 - i); ctx.fillText(v.toFixed(1), pL - 6, pT + (cH / 4) * i + 3); }
    ctx.setLineDash([4, 4]);
    if (warn >= yMin && warn <= yMax) { ctx.strokeStyle = 'rgba(234,179,8,0.3)'; ctx.beginPath(); ctx.moveTo(pL, toY(warn)); ctx.lineTo(W - pR, toY(warn)); ctx.stroke(); }
    if (crit >= yMin && crit <= yMax) { ctx.strokeStyle = 'rgba(239,68,68,0.3)'; ctx.beginPath(); ctx.moveTo(pL, toY(crit)); ctx.lineTo(W - pR, toY(crit)); ctx.stroke(); }
    ctx.setLineDash([]);
    if (simulated) ctx.setLineDash([6, 4]);
    ctx.beginPath();
    for (let i = 0; i < data.length; i++) { const x = toX(i), y = toY(data[i]); i === 0 ? ctx.moveTo(x, y) : ctx.lineTo(x, y); }
    ctx.strokeStyle = color; ctx.lineWidth = 2; ctx.lineJoin = 'round'; ctx.lineCap = 'round'; ctx.stroke();
    const lx = toX(data.length - 1), fx = toX(0);
    ctx.lineTo(lx, pT + cH); ctx.lineTo(fx, pT + cH); ctx.closePath();
    const g = ctx.createLinearGradient(0, pT, 0, pT + cH); g.addColorStop(0, color + '30'); g.addColorStop(1, color + '00');
    ctx.fillStyle = g; ctx.fill();
    const lv = data[data.length - 1], cy = toY(lv);
    ctx.beginPath(); ctx.arc(lx, cy, 4, 0, Math.PI * 2); ctx.fillStyle = color; ctx.fill();
    ctx.beginPath(); ctx.arc(lx, cy, 7, 0, Math.PI * 2); ctx.strokeStyle = color + '40'; ctx.lineWidth = 2; ctx.stroke();
    ctx.fillStyle = color; ctx.font = "bold 11px 'SF Mono','JetBrains Mono',Menlo,monospace"; ctx.textAlign = 'right'; ctx.fillText(lv.toFixed(1), W - pR - 2, cy - 8);
  }, [data, color, warn, crit, size, simulated]);

  return <div ref={containerRef} style={{ width: '100%', height: '100%' }}><canvas ref={canvasRef} style={{ display: 'block', width: '100%', height: '100%' }} /></div>;
}

// ── Big Number ──

function BigNumberChart({ value, unit, color: rawColor, warn, crit, decimals }: {
  value: number; unit: string; color: string; warn: number; crit: number; decimals: number;
}) {
  const color = rawColor.replace(/"/g, '');
  const st = value >= crit ? 'critical' : value >= warn ? 'warning' : 'normal';
  const stCol = st === 'critical' ? 'var(--crit)' : st === 'warning' ? 'var(--warn)' : color;
  const pct = warn > 0 ? Math.min((value / (crit || warn)) * 100, 100) : 0;

  return (
    <div style={{ display: 'flex', flexDirection: 'column', alignItems: 'center', justifyContent: 'center', height: '100%', padding: '10px 16px' }}>
      <div style={{ fontFamily: "var(--font-mono)", fontSize: 48, fontWeight: 800, color: stCol, lineHeight: 1.1 }}>
        {value.toFixed(decimals)}
      </div>
      {unit && <div style={{ fontSize: 14, color: 'var(--muted)', marginTop: 4, textTransform: 'uppercase', letterSpacing: '.06em' }}>{unit}</div>}
      <div style={{ width: '80%', height: 4, borderRadius: 2, background: 'var(--border)', marginTop: 12 }}>
        <div style={{ height: '100%', borderRadius: 2, background: stCol, width: `${pct}%`, transition: 'width 0.3s' }} />
      </div>
    </div>
  );
}

// ── Gauge ──

function GaugeChart({ value, min, max, warn, crit, color: rawColor, decimals, unit }: {
  value: number; min: number; max: number; warn: number; crit: number; color: string; decimals: number; unit: string;
}) {
  const color = rawColor.replace(/"/g, '');
  const containerRef = useRef<HTMLDivElement>(null)
  const canvasRef = useRef<HTMLCanvasElement>(null)
  const size = useCanvasSize(containerRef)

  useEffect(() => {
    const cv = canvasRef.current; if (!cv) return;
    const ctx = cv.getContext('2d')!
    const { width: W, height: H, dpr } = size
    if (W <= 0 || H <= 0) return

    cv.width = W * dpr; cv.height = H * dpr; ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    const cx = W / 2, cy = H * 0.78;
    const r = Math.min(W, H) * 0.38;
    ctx.clearRect(0, 0, W, H);

    const startAngle = Math.PI;
    const endAngle = 2 * Math.PI;
    const range = max - min || 1;
    const pct = Math.max(0, Math.min(1, (value - min) / range));
    const valueAngle = startAngle + pct * Math.PI;

    // Background arc
    ctx.beginPath();
    ctx.arc(cx, cy, r, startAngle, endAngle);
    ctx.strokeStyle = '#1a1a1e';
    ctx.lineWidth = 12;
    ctx.lineCap = 'round';
    ctx.stroke();

    // Colored arc segments
    const warnPct = Math.max(0, Math.min(1, (warn - min) / range));
    const critPct = Math.max(0, Math.min(1, (crit - min) / range));

    // Green arc
    ctx.beginPath();
    ctx.arc(cx, cy, r, startAngle, startAngle + warnPct * Math.PI);
    ctx.strokeStyle = '#22c55e';
    ctx.lineWidth = 12;
    ctx.lineCap = 'round';
    ctx.stroke();

    // Yellow arc
    if (warnPct < critPct) {
      ctx.beginPath();
      ctx.arc(cx, cy, r, startAngle + warnPct * Math.PI, startAngle + critPct * Math.PI);
      ctx.strokeStyle = '#eab308';
      ctx.lineWidth = 12;
      ctx.lineCap = 'butt';
      ctx.stroke();
    }

    // Red arc
    if (critPct < 1) {
      ctx.beginPath();
      ctx.arc(cx, cy, r, startAngle + critPct * Math.PI, endAngle);
      ctx.strokeStyle = '#ef4444';
      ctx.lineWidth = 12;
      ctx.lineCap = 'round';
      ctx.stroke();
    }

    // Needle
    const needleLen = r * 0.7;
    const nx = cx + Math.cos(valueAngle) * needleLen;
    const ny = cy + Math.sin(valueAngle) * needleLen;
    ctx.beginPath();
    ctx.moveTo(cx, cy);
    ctx.lineTo(nx, ny);
    ctx.strokeStyle = color;
    ctx.lineWidth = 2.5;
    ctx.lineCap = 'round';
    ctx.stroke();

    // Center dot
    ctx.beginPath();
    ctx.arc(cx, cy, 4, 0, Math.PI * 2);
    ctx.fillStyle = color;
    ctx.fill();

    // Value text
    ctx.fillStyle = color;
    ctx.font = "bold 20px 'SF Mono','JetBrains Mono',Menlo,monospace";
    ctx.textAlign = 'center';
    ctx.fillText(value.toFixed(decimals), cx, cy + 24);

    // Unit text
    if (unit) {
      ctx.fillStyle = '#71717a';
      ctx.font = "11px 'SF Pro Text',-apple-system,sans-serif";
      ctx.fillText(unit, cx, cy + 40);
    }

    // Min/max labels
    ctx.fillStyle = '#52525b';
    ctx.font = "10px 'SF Mono','JetBrains Mono',Menlo,monospace";
    ctx.textAlign = 'center';
    ctx.fillText(min.toFixed(0), cx - r - 4, cy + 14);
    ctx.fillText(max.toFixed(0), cx + r + 4, cy + 14);
  }, [value, min, max, warn, crit, color, decimals, unit, size]);

  return <div ref={containerRef} style={{ width: '100%', height: '100%' }}><canvas ref={canvasRef} style={{ display: 'block', width: '100%', height: '100%' }} /></div>;
}

// ── Image Card ──

function ImageCard({ imageUrl }: { imageUrl: string }) {
  if (!imageUrl) {
    return (
      <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'center', height: '100%', color: 'var(--muted)', fontSize: 12 }}>
        No image configured
      </div>
    );
  }
  return (
    <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'center', height: '100%', padding: 8, overflow: 'hidden' }}>
      <img src={imageUrl} alt="" style={{ maxWidth: '100%', maxHeight: '100%', objectFit: 'contain', borderRadius: 6 }} />
    </div>
  );
}

// ── Text (Markdown) ──

function TextCard({ text }: { text: string }) {
  const html = useMemo(() => {
    if (!text) return '';
    const raw = marked.parse(text, { async: false }) as string;
    return DOMPurify.sanitize(raw);
  }, [text]);

  if (!text) {
    return (
      <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'center', height: '100%', color: 'var(--muted)', fontSize: 12 }}>
        No text configured
      </div>
    );
  }

  return (
    <div
      style={{ padding: '12px 16px', overflow: 'auto', height: '100%', fontSize: 14, lineHeight: 1.6, color: 'var(--fg)' }}
      dangerouslySetInnerHTML={{ __html: html }}
      className="md-text"
    />
  );
}

// ── Dispatcher ──

export interface ChartProps {
  chartType: string;
  data: number[];
  color: string;
  warn: number;
  crit: number;
  min: number;
  max: number;
  decimals: number;
  unit: string;
  imageUrl?: string;
  textContent?: string;
  simulated?: boolean;
}

export function Chart({ chartType, data: rawData, color, warn, crit, min, max, decimals, unit, imageUrl, textContent, simulated }: ChartProps) {
  const data = rawData.filter(v => v != null && !Number.isNaN(v));
  const value = data.length > 0 ? data[data.length - 1] : 0;

  switch (chartType) {
    case 'bignumber':
      return <BigNumberChart value={value} unit={unit} color={color} warn={warn} crit={crit} decimals={decimals} />;
    case 'gauge':
      return <GaugeChart value={value} min={min} max={max} warn={warn} crit={crit} color={color} decimals={decimals} unit={unit} />;
    case 'image':
      return <ImageCard imageUrl={imageUrl || ''} />;
    case 'text':
      return <TextCard text={textContent || ''} />;
    default:
      return <SparklineChart data={data} color={color} warn={warn} crit={crit} simulated={simulated} />;
  }
}
