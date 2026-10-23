import type { WidgetInstance, ConnectionInstance, ScadaWidgetType } from '../widgets/types'

export interface ScadaLayoutState {
  widgets: WidgetInstance[]
  connections: ConnectionInstance[]
}

export type LayoutAction =
  | { type: 'ADD_WIDGET'; widget: WidgetInstance }
  | { type: 'REMOVE_WIDGET'; id: string }
  | { type: 'UPDATE_WIDGET'; id: string; updates: Partial<WidgetInstance> }
  | { type: 'UPDATE_WIDGET_CONFIG'; id: string; config: Record<string, any> }
  | { type: 'MOVE_WIDGET'; id: string; x: number; y: number }
  | { type: 'RESIZE_WIDGET'; id: string; x: number; y: number; width: number; height: number }
  | { type: 'ADD_CONNECTION'; connection: ConnectionInstance }
  | { type: 'REMOVE_CONNECTION'; id: string }
  | { type: 'UPDATE_CONNECTION'; id: string; updates: Partial<ConnectionInstance> }
  | { type: 'REORDER_WIDGET'; id: string; to: 'front' | 'back' }
  | { type: 'LOAD_LAYOUT'; widgets: WidgetInstance[]; connections: ConnectionInstance[] }
  | { type: 'CLEAR' }

export function layoutReducer(state: ScadaLayoutState, action: LayoutAction): ScadaLayoutState {
  switch (action.type) {
    case 'ADD_WIDGET':
      return { ...state, widgets: [...state.widgets, action.widget] }

    case 'REMOVE_WIDGET': {
      const id = action.id
      return {
        widgets: state.widgets.filter(w => w.id !== id),
        connections: state.connections.filter(c => c.fromWidgetId !== id && c.toWidgetId !== id),
      }
    }

    case 'UPDATE_WIDGET':
      return {
        ...state,
        widgets: state.widgets.map(w =>
          w.id === action.id ? { ...w, ...action.updates } : w,
        ),
      }

    case 'UPDATE_WIDGET_CONFIG': {
      const w = state.widgets.find(x => x.id === action.id)
      if (!w || Object.entries(action.config).every(([k, v]) => w.config?.[k] === v)) return state
      return {
        ...state,
        widgets: state.widgets.map(w =>
          w.id === action.id ? { ...w, config: { ...w.config, ...action.config } } : w,
        ),
      }
    }

    case 'MOVE_WIDGET': {
      const w = state.widgets.find(x => x.id === action.id)
      if (!w || (w.x === action.x && w.y === action.y)) return state
      return {
        ...state,
        widgets: state.widgets.map(w =>
          w.id === action.id ? { ...w, x: action.x, y: action.y } : w,
        ),
      }
    }

    case 'RESIZE_WIDGET': {
      const w = state.widgets.find(x => x.id === action.id)
      if (!w || (w.x === action.x && w.y === action.y && w.width === action.width && w.height === action.height)) return state
      return {
        ...state,
        widgets: state.widgets.map(w =>
          w.id === action.id
            ? { ...w, x: action.x, y: action.y, width: action.width, height: action.height }
            : w,
        ),
      }
    }

    case 'ADD_CONNECTION':
      return { ...state, connections: [...state.connections, action.connection] }

    case 'REMOVE_CONNECTION':
      return {
        ...state,
        connections: state.connections.filter(c => c.id !== action.id),
      }

    case 'UPDATE_CONNECTION':
      return {
        ...state,
        connections: state.connections.map(c => (c.id === action.id ? { ...c, ...action.updates } : c)),
      }

    case 'REORDER_WIDGET': {
      const idx = state.widgets.findIndex(x => x.id === action.id)
      if (idx < 0 || idx === (action.to === 'front' ? state.widgets.length - 1 : 0)) return state
      const w = state.widgets[idx]
      const rest = state.widgets.filter(x => x.id !== action.id)
      return { ...state, widgets: action.to === 'front' ? [...rest, w] : [w, ...rest] }
    }

    case 'LOAD_LAYOUT':
      return { widgets: action.widgets, connections: action.connections }

    case 'CLEAR':
      return { widgets: [], connections: [] }

    default:
      return state
  }
}

export function createWidget(
  type: ScadaWidgetType,
  sensorId: number | null,
  x: number,
  y: number,
  defaultWidth: number,
  defaultHeight: number,
): WidgetInstance {
  return {
    id: `w_${Date.now()}_${Math.random().toString(36).slice(2, 7)}`,
    type,
    sensorId,
    sensorIds: sensorId != null ? [sensorId] : [],
    x,
    y,
    width: defaultWidth,
    height: defaultHeight,
    config: {},
  }
}

export function createConnection(
  fromId: string,
  toId: string,
  connectionType: 'power' | 'data' | 'flow',
): ConnectionInstance {
  return {
    id: `c_${Date.now()}_${Math.random().toString(36).slice(2, 7)}`,
    fromWidgetId: fromId,
    toWidgetId: toId,
    connectionType,
    waypoints: [],
  }
}

// ── Undo / redo ──

export interface LayoutHistory {
  past: ScadaLayoutState[]
  present: ScadaLayoutState
  future: ScadaLayoutState[]
  /** Consecutive edits with the same key (typing in one field) collapse into one undo step. */
  lastKey: string | null
  /** Snapshot last loaded from / written to the server; `present !== saved` means unsaved changes. */
  saved: ScadaLayoutState
}

export type HistoryAction =
  | (LayoutAction & { coalesce?: string })
  | { type: 'UNDO' }
  | { type: 'REDO' }
  | { type: 'MARK_SAVED'; snapshot?: ScadaLayoutState }

const HISTORY_LIMIT = 100

export function historyReducer(h: LayoutHistory, action: HistoryAction): LayoutHistory {
  switch (action.type) {
    case 'UNDO': {
      if (h.past.length === 0) return h
      return { ...h, past: h.past.slice(0, -1), present: h.past[h.past.length - 1], future: [h.present, ...h.future], lastKey: null }
    }
    case 'REDO': {
      if (h.future.length === 0) return h
      return { ...h, past: [...h.past, h.present], present: h.future[0], future: h.future.slice(1), lastKey: null }
    }
    case 'LOAD_LAYOUT': {
      const present = layoutReducer(h.present, action)
      return { past: [], present, future: [], lastKey: null, saved: present }
    }
    case 'MARK_SAVED':
      return { ...h, saved: action.snapshot ?? h.present, lastKey: null }
    default: {
      const next = layoutReducer(h.present, action)
      if (next === h.present) return h
      const key = action.coalesce ?? null
      if (key && key === h.lastKey) return { ...h, present: next, future: [] }
      return { ...h, past: [...h.past, h.present].slice(-HISTORY_LIMIT), present: next, future: [], lastKey: key }
    }
  }
}

const EMPTY: ScadaLayoutState = { widgets: [], connections: [] }
export const emptyHistory: LayoutHistory = { past: [], present: EMPTY, future: [], lastKey: null, saved: EMPTY }
