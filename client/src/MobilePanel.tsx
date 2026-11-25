import { useEffect, useMemo, useState } from 'react'
import qrcode from 'qrcode-generator'
import {
  useCreatePairing, useMobileDevices, useMobileSettings, useRemoveMobileDevice,
  useSaveMobileSettings, type MobileDevice, type MobileSettings, type PairingResponse,
} from './queries/mobile'
import { btnStyle, inputStyle, labelStyle, labelTextStyle } from './styles'
import { fmtUnixSecs } from './unixTime'
import type { SensorConfig } from './types'

const sectionStyle: React.CSSProperties = { padding: '16px 0', borderBottom: '1px solid var(--border)' }
const h3Style: React.CSSProperties = { fontSize: 14, fontWeight: 700, marginBottom: 8 }
const noteStyle: React.CSSProperties = { fontSize: 13, color: 'var(--muted)', lineHeight: 1.5 }
const thStyle: React.CSSProperties = { textAlign: 'left', padding: '8px 12px', fontSize: 11, color: 'var(--muted)', textTransform: 'uppercase', letterSpacing: '.05em', fontWeight: 600, borderBottom: '1px solid var(--border)' }
const tdStyle: React.CSSProperties = { padding: '8px 12px', fontSize: 13, borderBottom: '1px solid var(--border)' }

/** The QR code of `text` as an SVG of dark modules on white (quiet zone included). */
function QrCode({ text, size = 240 }: { text: string; size?: number }) {
  const path = useMemo(() => {
    const qr = qrcode(0, 'M')
    qr.addData(text)
    qr.make()
    const n = qr.getModuleCount()
    let d = ''
    for (let r = 0; r < n; r++) {
      for (let c = 0; c < n; c++) {
        if (qr.isDark(r, c)) d += `M${c + 4},${r + 4}h1v1h-1z`
      }
    }
    return { d, n: n + 8 }
  }, [text])
  return (
    <svg width={size} height={size} viewBox={`0 0 ${path.n} ${path.n}`} shapeRendering="crispEdges" role="img" aria-label="Pairing QR code">
      <rect width={path.n} height={path.n} fill="#fff" />
      <path d={path.d} fill="#000" />
    </svg>
  )
}

function useSecondsLeft(until: number | undefined): number {
  const [now, setNow] = useState(() => Math.floor(Date.now() / 1000))
  useEffect(() => {
    if (!until) return
    const t = setInterval(() => setNow(Math.floor(Date.now() / 1000)), 1000)
    return () => clearInterval(t)
  }, [until])
  return until ? Math.max(0, until - now) : 0
}

function PairPhone() {
  const pairing = useCreatePairing()
  const [shown, setShown] = useState<PairingResponse | null>(null)
  const left = useSecondsLeft(shown?.expires_at)
  const expired = shown !== null && left === 0

  const show = () => pairing.mutate(undefined, { onSuccess: setShown })
  const reachable = shown && (shown.payload.urls.length > 0 || shown.payload.relay)

  return (
    <div style={sectionStyle}>
      <h3 style={h3Style}>Connect a phone</h3>
      <p style={noteStyle}>
        Install <b>Voltrus SCADA</b> from Google Play or the App Store, tap <i>Add site</i> and scan this code.
        The phone then acts as you: it sees what you see and can acknowledge alarms if your role allows it.
        The code works once, for 10 minutes.
      </p>
      {!shown || expired ? (
        <button style={{ ...btnStyle, marginTop: 12 }} onClick={show} disabled={pairing.isPending}>
          {pairing.isPending ? 'Creating…' : expired ? 'Show a new code' : 'Show QR code'}
        </button>
      ) : (
        <div style={{ display: 'flex', gap: 20, marginTop: 12, flexWrap: 'wrap', alignItems: 'flex-start' }}>
          <QrCode text={shown.link} />
          <div style={{ ...noteStyle, maxWidth: 360 }}>
            <div><b>{shown.payload.site}</b></div>
            <div>Expires in {Math.floor(left / 60)}:{String(left % 60).padStart(2, '0')}</div>
            <div style={{ marginTop: 8 }}>The phone tries these addresses:</div>
            <ul style={{ margin: '4px 0 0 18px' }}>
              {shown.payload.urls.map(u => <li key={u}><code>{u}</code></li>)}
              {shown.payload.relay && <li>the Voltrus push relay (from anywhere)</li>}
            </ul>
            {!reachable && (
              <div style={{ color: 'var(--crit)', marginTop: 8 }}>
                This page is open on the server itself, so the code has no address a phone can use.
                Open the HMI from another computer, or add the site's address under Settings below.
              </div>
            )}
            <div style={{ marginTop: 8 }}>
              {shown.push_available
                ? 'Alarm notifications: on.'
                : 'Alarm notifications need an Integrator licence and the push relay. Without them the app shows alarms and values while the phone is on the site network.'}
            </div>
          </div>
        </div>
      )}
      {pairing.error && <div style={{ color: 'var(--crit)', fontSize: 13, marginTop: 8 }}>{pairing.error.message}</div>}
    </div>
  )
}

function DeviceTable({ devices, showOwner, onRemove }: {
  devices: MobileDevice[]
  showOwner: boolean
  onRemove: (d: MobileDevice) => void
}) {
  if (devices.length === 0) return <div style={noteStyle}>No phones.</div>
  return (
    <div style={{ overflowX: 'auto' }}>
      <table style={{ width: '100%', borderCollapse: 'collapse' }}>
        <thead>
          <tr>
            <th style={thStyle}>Phone</th>
            {showOwner && <th style={thStyle}>User</th>}
            <th style={thStyle}>Notifications</th>
            <th style={thStyle}>Last seen</th>
            <th style={thStyle} />
          </tr>
        </thead>
        <tbody>
          {devices.map(d => (
            <tr key={d.id}>
              <td style={tdStyle}>{d.name} <span style={{ color: 'var(--muted)' }}>({d.platform === 'ios' ? 'iPhone' : 'Android'})</span></td>
              {showOwner && <td style={tdStyle}>{d.username}</td>}
              <td style={tdStyle}>{d.push ? `${d.min_priority} and above` : 'off'}</td>
              <td style={tdStyle}>{d.last_seen_at ? fmtUnixSecs(d.last_seen_at) : '—'}</td>
              <td style={{ ...tdStyle, textAlign: 'right' }}>
                <button style={{ ...btnStyle, color: 'var(--crit)' }} onClick={() => onRemove(d)}>Remove</button>
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  )
}

function Devices({ all }: { all: boolean }) {
  const { data: devices = [], isLoading } = useMobileDevices(all)
  const remove = useRemoveMobileDevice()
  const [confirm, setConfirm] = useState<MobileDevice | null>(null)
  return (
    <div style={sectionStyle}>
      <h3 style={h3Style}>{all ? 'All paired phones' : 'My phones'}</h3>
      {isLoading ? <div style={noteStyle}>Loading…</div> : (
        <DeviceTable devices={devices} showOwner={all} onRemove={setConfirm} />
      )}
      {confirm && (
        <div style={{ marginTop: 12, fontSize: 13, display: 'flex', gap: 8, alignItems: 'center', flexWrap: 'wrap' }}>
          <span>Remove <b>{confirm.name}</b>{all ? ` of ${confirm.username}` : ''}? It stops receiving alarms and can no longer open this site.</span>
          <button style={{ ...btnStyle, color: 'var(--crit)' }} disabled={remove.isPending}
            onClick={() => remove.mutate(confirm.id, { onSuccess: () => setConfirm(null) })}>Remove</button>
          <button style={btnStyle} onClick={() => setConfirm(null)}>Cancel</button>
        </div>
      )}
      {remove.error && <div style={{ color: 'var(--crit)', fontSize: 13, marginTop: 8 }}>{remove.error.message}</div>}
    </div>
  )
}

function SettingsForm({ sensors }: { sensors: SensorConfig[] }) {
  const { data } = useMobileSettings(true)
  const save = useSaveMobileSettings()
  const [draft, setDraft] = useState<MobileSettings | null>(null)
  const [tagToAdd, setTagToAdd] = useState('')
  const [saved, setSaved] = useState(false)

  useEffect(() => {
    if (data && !draft) {
      setDraft({ site_name: data.site_name, push_details: data.push_details, pinned_tags: data.pinned_tags, public_urls: data.public_urls })
    }
  }, [data, draft])

  if (!data || !draft) return null
  const tagName = (key: string) => sensors.find(s => s.key === key)?.name ?? key
  const set = (patch: Partial<MobileSettings>) => { setDraft({ ...draft, ...patch }); setSaved(false) }

  return (
    <div style={sectionStyle}>
      <h3 style={h3Style}>Settings</h3>
      <div style={{ ...noteStyle, marginBottom: 12 }}>
        Push relay: {!data.push_licensed
          ? 'needs an Integrator or Enterprise licence.'
          : data.relay
            ? (data.relay.connected ? 'connected.' : 'not connected right now.')
            : 'not set up.'}
        {' '}Up to {data.device_limit} phones per site.
      </div>
      <div style={{ display: 'grid', gap: 12, maxWidth: 560 }}>
        <label style={labelStyle}>
          <span style={labelTextStyle}>Site name on the phone</span>
          <input style={inputStyle} value={draft.site_name} placeholder={data.effective_site_name}
            onChange={e => set({ site_name: e.target.value })} />
        </label>
        <label style={{ display: 'flex', gap: 8, alignItems: 'center', fontSize: 13 }}>
          <input type="checkbox" checked={draft.push_details} onChange={e => set({ push_details: e.target.checked })} />
          Show the alarm text in notifications (off: only “new alarm at this site”, details open in the app)
        </label>
        <div style={labelStyle}>
          <span style={labelTextStyle}>Key tags on the phone's site screen</span>
          {draft.pinned_tags.map((key, i) => (
            <div key={key} style={{ display: 'flex', gap: 8, alignItems: 'center', fontSize: 13 }}>
              <span style={{ flex: 1 }}>{tagName(key)} <span style={{ color: 'var(--muted)' }}>{key}</span></span>
              <button style={btnStyle} disabled={i === 0} aria-label="Move up"
                onClick={() => { const t = [...draft.pinned_tags];[t[i - 1], t[i]] = [t[i], t[i - 1]]; set({ pinned_tags: t }) }}>↑</button>
              <button style={btnStyle} onClick={() => set({ pinned_tags: draft.pinned_tags.filter(k => k !== key) })}>Remove</button>
            </div>
          ))}
          <div style={{ display: 'flex', gap: 8 }}>
            <select style={{ ...inputStyle, flex: 1 }} value={tagToAdd} onChange={e => setTagToAdd(e.target.value)}>
              <option value="">Add a tag…</option>
              {sensors.filter(s => !draft.pinned_tags.includes(s.key)).map(s => (
                <option key={s.id} value={s.key}>{s.name} ({s.key})</option>
              ))}
            </select>
            <button style={btnStyle} disabled={!tagToAdd}
              onClick={() => { set({ pinned_tags: [...draft.pinned_tags, tagToAdd] }); setTagToAdd('') }}>Add</button>
          </div>
        </div>
        <label style={labelStyle}>
          <span style={labelTextStyle}>Other addresses of this site (one per line)</span>
          <textarea style={{ ...inputStyle, minHeight: 64, fontFamily: 'var(--font-mono)' }}
            value={draft.public_urls.join('\n')}
            placeholder="http://10.8.0.5:3000"
            onChange={e => set({ public_urls: e.target.value.split('\n') })} />
          <span style={{ fontSize: 12, color: 'var(--muted)' }}>
            For example the address on the plant VPN. The phone tries these before the push relay.
          </span>
        </label>
        <div style={{ display: 'flex', gap: 8, alignItems: 'center' }}>
          <button style={btnStyle} disabled={save.isPending}
            onClick={() => save.mutate(
              { ...draft, public_urls: draft.public_urls.map(u => u.trim()).filter(Boolean) },
              { onSuccess: s => { setDraft(s); setSaved(true) } },
            )}>
            {save.isPending ? 'Saving…' : 'Save'}
          </button>
          {saved && <span style={{ fontSize: 13, color: 'var(--ok)' }}>Saved</span>}
          {save.error && <span style={{ fontSize: 13, color: 'var(--crit)' }}>{save.error.message}</span>}
        </div>
      </div>
    </div>
  )
}

/** Pairing phones with the Voltrus SCADA app, the paired phones, and (for configurers) the app settings. */
export function MobilePanel({ sensors, canConfigure, isStrictAdmin }: {
  sensors: SensorConfig[]
  canConfigure: boolean
  isStrictAdmin: boolean
}) {
  return (
    <div>
      <PairPhone />
      <Devices all={false} />
      {isStrictAdmin && <Devices all />}
      {canConfigure && <SettingsForm sensors={sensors} />}
    </div>
  )
}
