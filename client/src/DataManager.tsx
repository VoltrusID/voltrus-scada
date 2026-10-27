import { useState, useEffect, useRef } from 'react';
import { useNavigate, useRouterState } from '@tanstack/react-router';
import { SensorManager } from './SensorManager';
import { ModbusDeviceManager } from './ModbusDeviceManager';
import { OpcUaDeviceManager } from './OpcUaDeviceManager';
import { UserManager } from './UserManager';
import { ApiKeyManager } from './ApiKeyManager';
import { NotificationSettingsPanel } from './NotificationSettingsPanel';
import { PushSettings } from './PushSettings';
import { OeePanel } from './OeePanel';
import { AnalyticsPanel } from './AnalyticsPanel';
import { RecipeManager } from './RecipeManager';
import { S7DeviceManager } from './S7DeviceManager';
import { EnipDeviceManager } from './EnipDeviceManager';
import { Dnp3DeviceManager } from './Dnp3DeviceManager';
import { BacnetDeviceManager } from './BacnetDeviceManager';
import { RoleManager } from './RoleManager';
import { useLicense } from './LicenseContext';
import { edition, mergeTabs, type DataManagerTab } from './edition';
import type { SensorConfig } from './types';

const CORE_TAB_IDS = ['sources', 'modbus', 'opcua', 's7', 'enip', 'dnp3', 'bacnet', 'users', 'notifications', 'api-keys', 'oee', 'analytics', 'recipes', 'roles'] as const;

interface NavItem {
  id: string
  label: string
  icon: React.ReactNode
  admin?: boolean
  strictAdmin?: boolean
}

interface NavGroup {
  label: string
  items: NavItem[]
}

const ICON_SIZE = 16

const CORE_GROUPS: NavGroup[] = [
  {
    label: 'Data',
    items: [
      { id: 'sources', label: 'Sources', icon: <svg width={ICON_SIZE} height={ICON_SIZE} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2"><circle cx="12" cy="12" r="3"/><path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 0 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 0 1-2.83-2.83l.06-.06A1.65 1.65 0 0 0 4.68 15a1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 0 1 2.83-2.83l.06.06A1.65 1.65 0 0 0 9 4.68a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 0 1 2.83 2.83l-.06.06A1.65 1.65 0 0 0 19.4 9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z"/></svg> },
    ],
  },
  {
    label: 'Protocols',
    items: [
      { id: 'modbus', label: 'Modbus', icon: <svg width={ICON_SIZE} height={ICON_SIZE} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2"><rect x="2" y="7" width="20" height="14" rx="2" ry="2"/><path d="M16 21V5a2 2 0 0 0-2-2h-4a2 2 0 0 0-2 2v16"/></svg> },
      { id: 'opcua', label: 'OPC-UA', admin: true, icon: <svg width={ICON_SIZE} height={ICON_SIZE} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2"><path d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z"/></svg> },
      { id: 's7', label: 'S7', admin: true, icon: <svg width={ICON_SIZE} height={ICON_SIZE} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2"><polygon points="13 2 3 14 12 14 11 22 21 10 12 10 13 2"/></svg> },
      { id: 'enip', label: 'ENIP', admin: true, icon: <svg width={ICON_SIZE} height={ICON_SIZE} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2"><rect x="2" y="2" width="20" height="8" rx="2" ry="2"/><rect x="2" y="14" width="20" height="8" rx="2" ry="2"/><line x1="6" y1="6" x2="6.01" y2="6"/><line x1="6" y1="18" x2="6.01" y2="18"/></svg> },
      { id: 'dnp3', label: 'DNP3', admin: true, icon: <svg width={ICON_SIZE} height={ICON_SIZE} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2"><path d="M12 22s8-4 8-10V5l-8-3-8 3v7c0 6 8 10 8 10z"/></svg> },
      { id: 'bacnet', label: 'BACnet', admin: true, icon: <svg width={ICON_SIZE} height={ICON_SIZE} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2"><path d="M3 9l9-7 9 7v11a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z"/><polyline points="9 22 9 12 15 12 15 22"/></svg> },
    ],
  },
  {
    label: 'Insights',
    items: [
      { id: 'oee', label: 'OEE', icon: <svg width={ICON_SIZE} height={ICON_SIZE} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2"><line x1="18" y1="20" x2="18" y2="10"/><line x1="12" y1="20" x2="12" y2="4"/><line x1="6" y1="20" x2="6" y2="14"/></svg> },
      { id: 'analytics', label: 'Analytics', icon: <svg width={ICON_SIZE} height={ICON_SIZE} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2"><polyline points="22 12 18 12 15 21 9 3 6 12 2 12"/></svg> },
      { id: 'recipes', label: 'Recipes', admin: true, icon: <svg width={ICON_SIZE} height={ICON_SIZE} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2"><path d="M4 19.5A2.5 2.5 0 0 1 6.5 17H20"/><path d="M6.5 2H20v20H6.5A2.5 2.5 0 0 1 4 19.5v-15A2.5 2.5 0 0 1 6.5 2z"/></svg> },
    ],
  },
  {
    label: 'Admin',
    items: [
      { id: 'users', label: 'Users', admin: true, strictAdmin: true, icon: <svg width={ICON_SIZE} height={ICON_SIZE} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2"><path d="M17 21v-2a4 4 0 0 0-4-4H5a4 4 0 0 0-4 4v2"/><circle cx="9" cy="7" r="4"/><path d="M23 21v-2a4 4 0 0 0-3-3.87"/><path d="M16 3.13a4 4 0 0 1 0 7.75"/></svg> },
      { id: 'roles', label: 'Roles', admin: true, strictAdmin: true, icon: <svg width={ICON_SIZE} height={ICON_SIZE} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2"><path d="M12 22s8-4 8-10V5l-8-3-8 3v7c0 6 8 10 8 10z"/></svg> },
      { id: 'notifications', label: 'Notifications', admin: true, icon: <svg width={ICON_SIZE} height={ICON_SIZE} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2"><path d="M18 8A6 6 0 0 0 6 8c0 7-3 9-3 9h18s-3-2-3-9"/><path d="M13.73 21a2 2 0 0 1-3.46 0"/></svg> },
      { id: 'api-keys', label: 'API Keys', admin: true, strictAdmin: true, icon: <svg width={ICON_SIZE} height={ICON_SIZE} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2"><path d="M21 2l-2 2m-7.61 7.61a5.5 5.5 0 1 1-7.778 7.778 5.5 5.5 0 0 1 7.777-7.777zm0 0L15.5 7.5m0 0l3 3L22 7l-3-3m-3.5 3.5L19 4"/></svg> },
    ],
  },
]

const LICENSE_ICON = <svg width={ICON_SIZE} height={ICON_SIZE} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2"><rect x="3" y="4" width="18" height="16" rx="2"/><path d="M7 9h10M7 13h6"/></svg>

// Tabs contributed by the enterprise module (empty in Community builds).
const EXTRA_TABS: DataManagerTab[] = [
  ...(edition.LicensePanel
    ? [{
        id: 'license', label: 'License', group: 'Admin', admin: true, strictAdmin: true, icon: LICENSE_ICON,
        render: () => edition.LicensePanel ? <edition.LicensePanel /> : null,
      }]
    : []),
  ...(edition.dataManagerTabs ?? []),
]

const GROUPS = mergeTabs(CORE_GROUPS, EXTRA_TABS)

const isTab = (v: unknown): v is string =>
  typeof v === 'string' && (CORE_TAB_IDS.some(id => id === v) || EXTRA_TABS.some(t => t.id === v))

const SIDEBAR_WIDTH = 200
const SIDEBAR_ITEM: React.CSSProperties = {
  display: 'flex', alignItems: 'center', gap: 10,
  width: '100%', padding: '7px 12px',
  fontSize: 13, fontWeight: 500,
  border: 'none', borderRadius: 6,
  background: 'transparent',
  color: 'var(--muted)',
  cursor: 'pointer',
  textAlign: 'left' as const,
  transition: 'background 0.1s, color 0.1s',
}
const SIDEBAR_ITEM_ACTIVE: React.CSSProperties = {
  ...SIDEBAR_ITEM,
  background: 'rgba(34,197,94,.08)',
  color: 'var(--cyan)',
  fontWeight: 600,
}
const GROUP_LABEL: React.CSSProperties = {
  fontSize: 10, fontWeight: 700, textTransform: 'uppercase', letterSpacing: '.08em',
  color: 'var(--text-muted)', padding: '4px 12px', marginTop: 12, marginBottom: 2,
}
const NARROW_QUERY = '(max-width: 720px)'
const SECTION_SELECT: React.CSSProperties = {
  width: '100%', padding: '8px 10px',
  fontSize: 16, fontWeight: 500,
  border: '1px solid var(--border)', borderRadius: 6,
  background: 'var(--bg)', color: 'var(--fg)',
  fontFamily: 'var(--font-ui)',
}

/** True on phone-width screens, where the section sidebar would take half the screen. */
function useNarrow(): boolean {
  const [narrow, setNarrow] = useState(() => window.matchMedia(NARROW_QUERY).matches);
  useEffect(() => {
    const mq = window.matchMedia(NARROW_QUERY);
    const onChange = () => setNarrow(mq.matches);
    mq.addEventListener('change', onChange);
    return () => mq.removeEventListener('change', onChange);
  }, []);
  return narrow;
}

export function DataManager({ sensors, onSave, onDelete, onAdd, onDuplicate, onClose, shortcuts, isAdmin, isStrictAdmin }: {
  sensors: SensorConfig[];
  onSave: (s: SensorConfig, shortcut?: string | null) => void;
  onDelete: (id: number) => void;
  onAdd: (s: Partial<SensorConfig>) => void;
  onDuplicate: (s: SensorConfig) => void;
  onClose: () => void;
  shortcuts: Record<number, string>;
  isAdmin?: boolean;
  isStrictAdmin?: boolean;
}) {
  const navigate = useNavigate();
  const routerSearch = useRouterState({ select: s => s.location.search }) as { tab?: string };
  const tab: string = isTab(routerSearch.tab) ? routerSearch.tab : 'sources';
  const setTab = (t: string) => navigate({ to: '/data', search: prev => ({ ...prev, tab: t }) });
  const modalRef = useRef<HTMLDivElement>(null);
  const license = useLicense();
  const narrow = useNarrow();
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => { if (e.key === 'Escape') onClose(); };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [onClose]);

  // Focus trap
  useEffect(() => {
    const modal = modalRef.current;
    if (!modal) return;

    const handleTabKey = (e: KeyboardEvent) => {
      if (e.key !== 'Tab') return;
      const focusableSelector = 'button, [href], input, select, textarea, [tabindex]:not([tabindex="-1"])';
      const focusableElements = modal.querySelectorAll<HTMLElement>(focusableSelector);
      if (focusableElements.length === 0) return;

      const firstEl = focusableElements[0];
      const lastEl = focusableElements[focusableElements.length - 1];

      if (e.shiftKey) {
        if (document.activeElement === firstEl) {
          e.preventDefault();
          lastEl.focus();
        }
      } else {
        if (document.activeElement === lastEl) {
          e.preventDefault();
          firstEl.focus();
        }
      }
    };

    document.addEventListener('keydown', handleTabKey);
    return () => document.removeEventListener('keydown', handleTabKey);
  }, []);

  // Find active label
  const extraTab = EXTRA_TABS.find(t => t.id === tab)
  const activeLabel = GROUPS.flatMap(g => g.items).find(i => i.id === tab)?.label ?? 'Data'
  const visibleGroups = GROUPS
    .map(group => ({
      ...group,
      items: group.items.filter(i => !(i.strictAdmin && !isStrictAdmin) && !(i.admin && !isAdmin)),
    }))
    .filter(group => group.items.length > 0)

  return (
    <div ref={modalRef} role="dialog" aria-label="Data Manager" aria-modal="true" style={{ position: 'fixed', inset: 0, background: 'var(--bg)', zIndex: 50, display: 'flex', flexDirection: 'column' }}>
      {/* Top bar */}
      <div style={{
        display: 'flex', alignItems: 'center', gap: 12,
        padding: '0 16px', height: 48,
        borderBottom: '1px solid var(--border)',
        background: 'var(--card)',
        flexShrink: 0,
      }}>
        <button onClick={onClose} style={{
          display: 'flex', alignItems: 'center', gap: 6,
          padding: '5px 10px', borderRadius: 6,
          border: '1px solid var(--border)', background: 'var(--card)',
          color: 'var(--muted)', cursor: 'pointer', fontSize: 12, fontWeight: 600,
          fontFamily: 'var(--font-ui)',
        }}>
          <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2"><path d="M19 12H5M12 19l-7-7 7-7" /></svg>
          Back
        </button>
        <h1 style={{ fontSize: 15, fontWeight: 700, color: 'var(--fg)' }}>Data</h1>
        <span style={{ fontSize: 12, color: 'var(--muted)', fontWeight: 500 }}>{activeLabel}</span>
      </div>

      <div style={{ flex: 1, display: 'flex', flexDirection: narrow ? 'column' : 'row', minHeight: 0 }}>
        {narrow ? (
          <nav aria-label="Data sections" style={{
            flexShrink: 0,
            borderBottom: '1px solid var(--border)',
            background: 'var(--card)',
            padding: '8px 12px',
          }}>
            <select aria-label="Section" value={tab} onChange={e => setTab(e.target.value)} style={SECTION_SELECT}>
              {visibleGroups.map(group => (
                <optgroup key={group.label} label={group.label}>
                  {group.items.map(item => <option key={item.id} value={item.id}>{item.label}</option>)}
                </optgroup>
              ))}
            </select>
          </nav>
        ) : (
          <nav style={{
            width: SIDEBAR_WIDTH,
            flexShrink: 0,
            borderRight: '1px solid var(--border)',
            background: 'var(--card)',
            overflowY: 'auto',
            padding: '8px 8px',
          }}>
            {visibleGroups.map(group => (
              <div key={group.label}>
                <div style={GROUP_LABEL}>{group.label}</div>
                {group.items.map(item => (
                  <button
                    key={item.id}
                    onClick={() => setTab(item.id)}
                    style={tab === item.id ? SIDEBAR_ITEM_ACTIVE : SIDEBAR_ITEM}
                    onMouseEnter={e => { if (tab !== item.id) e.currentTarget.style.background = 'var(--hover-overlay)' }}
                    onMouseLeave={e => { if (tab !== item.id) e.currentTarget.style.background = 'transparent' }}
                  >
                    <span style={{ width: 16, height: 16, display: 'flex', alignItems: 'center', justifyContent: 'center', flexShrink: 0, opacity: tab === item.id ? 1 : 0.5 }}>{item.icon}</span>
                    <span style={{ flex: 1 }}>{item.label}</span>
                  </button>
                ))}
              </div>
            ))}
          </nav>
        )}

        {/* Content */}
        <div style={{ flex: 1, display: 'flex', flexDirection: 'column', minHeight: 0, minWidth: 0, overflow: 'auto', padding: narrow ? 12 : 16 }}>
          {tab === 'sources' && (
            <SensorManager
              sensors={sensors} onSave={onSave} onDelete={onDelete}
              onAdd={onAdd} onDuplicate={onDuplicate} onClose={onClose}
              shortcuts={shortcuts} embedded
            />
          )}
          {tab === 'modbus' && (
            <ModbusDeviceManager onClose={onClose} embedded isStrictAdmin={isStrictAdmin} />
          )}
          {tab === 'users' && isStrictAdmin && (
            <UserManager onClose={onClose} embedded />
          )}
          {tab === 'notifications' && isAdmin && (
            <>
              <PushSettings />
              <NotificationSettingsPanel onClose={onClose} embedded />
            </>
          )}
          {tab === 'api-keys' && isStrictAdmin && (
            <ApiKeyManager onClose={onClose} embedded />
          )}
          {tab === 'opcua' && isAdmin && (
            <OpcUaDeviceManager onClose={onClose} embedded />
          )}
          {tab === 'oee' && (
            <OeePanel />
          )}
          {tab === 'analytics' && (
            <AnalyticsPanel />
          )}
          {tab === 'recipes' && isAdmin && (
            <RecipeManager />
          )}
          {tab === 's7' && isAdmin && (
            <S7DeviceManager onClose={onClose} embedded />
          )}
          {tab === 'enip' && isAdmin && (
            <EnipDeviceManager onClose={onClose} embedded />
          )}
          {tab === 'dnp3' && isAdmin && (
            <Dnp3DeviceManager onClose={onClose} embedded />
          )}
          {tab === 'bacnet' && isAdmin && (
            <BacnetDeviceManager onClose={onClose} embedded isStrictAdmin={isStrictAdmin} />
          )}
          {extraTab && (!extraTab.strictAdmin || isStrictAdmin) && (!extraTab.admin || isAdmin) && extraTab.render({ onClose, license })}
          {tab === 'roles' && isStrictAdmin && (
            <RoleManager onClose={onClose} embedded />
          )}
        </div>
      </div>
    </div>
  );
}
