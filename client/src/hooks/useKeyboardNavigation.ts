import { useEffect, useMemo } from 'react'
import { normalizeKeyEvent } from '../styles'
import type { SensorConfig } from '../types'

export function useKeyboardNavigation({
  shortcuts, sensors, screens, navShortcuts, effectiveNavShortcuts,
  setSelectedSensor, setActiveScreen, setViewMode, setMode, setShowAlarmPanel, mode,
}: {
  shortcuts: Record<number, string>
  sensors: SensorConfig[]
  screens: { id: string }[]
  navShortcuts: Record<string, string>
  effectiveNavShortcuts: Record<string, string>
  setSelectedSensor: (id: number | null) => void
  setActiveScreen: (id: string) => void
  setViewMode: (mode: 'cards' | 'canvas' | 'twin' | 'scada') => void
  setMode: (mode: 'live' | 'history' | 'system' | 'flows' | 'tags' | 'assets' | 'scripts') => void
  setShowAlarmPanel: (v: boolean | ((prev: boolean) => boolean)) => void
  mode: 'live' | 'history' | 'system' | 'flows' | 'tags' | 'assets' | 'scripts'
}) {
  const shortcutToSensorId = useMemo(() => {
    const map = new Map<string, number>();
    const normalize = (s: string) => s.replace(/\bCtrl\b/gi, 'Mod').replace(/\bMeta\b/gi, 'Mod').toLowerCase();
    for (const [id, sc] of Object.entries(shortcuts)) {
      const sensor = sensors.find(s => s.id === Number(id));
      if (sensor) map.set(normalize(sc), sensor.id);
    }
    return map;
  }, [shortcuts, sensors]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const el = e.target as HTMLElement | null;
      if (!el) return;
      if (el.tagName === 'INPUT' || el.tagName === 'TEXTAREA' || el.tagName === 'SELECT' || el.isContentEditable) return;
      const combo = normalizeKeyEvent(e).toLowerCase();

      // Nav shortcuts
      const navEntry = Object.entries(effectiveNavShortcuts).find(([, v]) => v && v.replace(/\bCtrl\b/gi, 'Mod').replace(/\bMeta\b/gi, 'Mod').toLowerCase() === combo);
      if (navEntry) {
        e.preventDefault();
        setSelectedSensor(null);
        const [target] = navEntry;
        if (target.startsWith('screen:')) {
          const screenId = target.slice(7);
          if (screenId === 'system') {
            setMode('system');
            setActiveScreen('all');
          } else {
            setActiveScreen(screenId);
            if (mode === 'system') setMode('live');
          }
        } else if (target.startsWith('view:')) {
          setViewMode(target.slice(5) as 'cards' | 'canvas' | 'twin' | 'scada');
        } else if (target.startsWith('mode:')) {
          setMode(target.slice(5) as 'live' | 'history');
        } else if (target === 'action:alarms') {
          setShowAlarmPanel(v => !v);
        }
        return;
      }

      // Sensor detail shortcuts
      const sensorId = shortcutToSensorId.get(combo);
      if (sensorId != null) {
        e.preventDefault();
        setSelectedSensor(sensorId);
      }
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [shortcutToSensorId, effectiveNavShortcuts, mode, setSelectedSensor, setActiveScreen, setViewMode, setMode, setShowAlarmPanel]);
}
