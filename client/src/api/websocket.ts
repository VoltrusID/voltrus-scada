import { encode, decode } from '@msgpack/msgpack';

export interface TagUpdate {
  tag_id: string;
  value: number;
  /** The tag's status (normal/warning/critical/offline), despite the name. */
  quality: string;
  ts: number;
  simulated: boolean;
  /** Per-value data quality; absent from older servers. */
  data_quality?: string;
  /** When the value was read from the device, Unix ms. */
  ts_ms?: number;
}

export interface TagUpdateBatch {
  updates: TagUpdate[];
}

export interface CommandAck {
  command_id: string;
  status: string;
  device_response?: string;
  latency_ms?: number;
  /** Set when the server wants this write electronically signed (status `signature_required` or `rejected`). */
  signature_required?: { action: string; meaning: string };
}

export interface DeviceStatus {
  device_id: string;
  online: boolean;
  latency_ms?: number;
}

export interface AlarmEvent {
  alarm_id: number;
  tag_id: string;
  severity: string;
  message: string;
  value: number;
  ts: number;
}

/** An existing alarm changed state; `actor` is null when the system did it. */
export interface AlarmUpdateEvent {
  alarm_id: number;
  tag_id: string;
  event: 'acknowledged' | 'resolved' | 'shelved' | 'unshelved';
  state: string;
  actor: string | null;
  ts: number;
}

/** Live collaboration frames from the server (docs/collaboration.md). */
export interface DesignLimits { max_update: number; max_rate: number; max_doc: number; max_awareness: number; flush_ms: number; idle_ms?: number }
export interface DesignUser { user: string; color: string }
export interface DesignSessionInfo { screen: string; users: DesignUser[] }
export type DesignServerMessage =
  | { type: 'DesignJoined'; screen: string; epoch: string; resync: boolean; user: string; color: string; protocol: number; limits: DesignLimits }
  | { type: 'DesignSync'; screen: string; data: Uint8Array }
  | { type: 'DesignAwareness'; screen: string; data: Uint8Array }
  | { type: 'DesignError'; screen: string; code: string; message: string; reload: boolean }
  | { type: 'DesignRejected'; screen: string; objects: { object_type: string; object_id: string; reason: string }[] }
  | { type: 'DesignFlushed'; screen: string; seq: number; at_ms: number; users: string[]; acked?: number }
  | { type: 'DesignSessions'; sessions: DesignSessionInfo[] }
  | { type: 'DesignComment'; screen: string; event: 'created' | 'updated' | 'deleted'; comment: Record<string, unknown> }
export type DesignClientMessage =
  | { type: 'DesignJoin'; screen: string; protocol: number; epoch?: string | null }
  | { type: 'DesignSync'; screen: string; data: Uint8Array }
  | { type: 'DesignAwareness'; screen: string; data: Uint8Array }
  | { type: 'DesignLeave'; screen: string }
  | { type: 'DesignFlush'; screen: string }

type DesignCallback = (msg: DesignServerMessage) => void;

type TagUpdateCallback = (update: TagUpdate) => void;
type TagBatchCallback = (updates: TagUpdate[]) => void;
type CommandAckCallback = (ack: CommandAck) => void;
type AlarmCallback = (alarm: AlarmEvent) => void;
type AlarmUpdateCallback = (update: AlarmUpdateEvent) => void;
type DeviceStatusCallback = (status: DeviceStatus) => void;
type DisconnectCallback = () => void;

/** How the live data reaches the browser: the WebSocket, or server-sent events when it cannot. */
export type Transport = 'ws' | 'sse';

type TransportCallback = (transport: Transport) => void;

/** Consecutive WebSocket attempts that end before the server says anything, then the SSE fallback starts. */
export const WS_FAILURES_BEFORE_FALLBACK = 3;
/** While on SSE, how often a WebSocket is tried again. */
export const SSE_PROBE_INTERVAL_MS = 60_000;
/**
 * Reconnect delays are spread by this fraction either way, so the clients of a
 * restarted server (a control room of kiosks) do not all come back at once.
 */
export const RECONNECT_JITTER = 0.3;
/** How often an open WebSocket sends the server an application-level `Ping`. */
export const KEEPALIVE_INTERVAL_MS = 20_000;
/**
 * A socket whose server answers pings and has said nothing for this long is
 * treated as dead (a half-open TCP connection after a network change) and replaced.
 */
export const STALE_AFTER_MS = 3 * KEEPALIVE_INTERVAL_MS;

/** `ms` spread by up to ±RECONNECT_JITTER. */
export function jittered(ms: number, rand: () => number = Math.random): number {
  return Math.round(ms * (1 - RECONNECT_JITTER + 2 * RECONNECT_JITTER * rand()));
}

/** Event names the server uses on the SSE stream: the `type` of each message. */
const SSE_EVENTS = [
  'AuthOk', 'AuthError', 'TagUpdate', 'TagUpdateBatch', 'CommandAck',
  'DeviceStatus', 'Alarm', 'AlarmUpdate', 'HaStatus',
] as const;

function addUnique<T>(list: T[], item: T): void {
  if (!list.includes(item)) list.push(item);
}

/**
 * The live data connection. It uses the WebSocket (MessagePack). When the
 * WebSocket cannot be established, as behind a proxy that blocks upgrades, it
 * falls back to the server's `text/event-stream` endpoint (JSON, read-only) and
 * tries the WebSocket again every minute, switching back as soon as one works.
 */
export class VoltrusWebSocket {
  private ws: WebSocket | null = null;
  private es: EventSource | null = null;
  private transport: Transport = 'ws';
  private sseUp = false;
  private wsFailures = 0;
  private probe: WebSocket | null = null;
  private probeTimer: ReturnType<typeof setTimeout> | null = null;
  private reconnectTimer: ReturnType<typeof setTimeout> | null = null;
  private reconnectDelay = 1000;
  private maxReconnectDelay = 30000;
  private tagUpdateCallbacks: TagUpdateCallback[] = [];
  private tagBatchCallbacks: TagBatchCallback[] = [];
  private commandAckCallbacks: CommandAckCallback[] = [];
  private alarmCallbacks: AlarmCallback[] = [];
  private alarmUpdateCallbacks: AlarmUpdateCallback[] = [];
  private deviceStatusCallbacks: DeviceStatusCallback[] = [];
  private disconnectCallbacks: DisconnectCallback[] = [];
  private transportCallbacks: TransportCallback[] = [];
  private designCallbacks: DesignCallback[] = [];
  private url = '';
  private sseUrl = '';
  private token = '';
  private batchBuffer: TagUpdate[] = [];
  private batchTimer: ReturnType<typeof setTimeout> | null = null;
  private connectCallbacks: (() => void)[] = [];
  private keepaliveTimer: ReturnType<typeof setTimeout> | null = null;
  private staleTimer: ReturnType<typeof setTimeout> | null = null;
  /**
   * Set once the server answered a `Ping`. Servers that never answer get no
   * staleness check: a quiet plant (no subscribed tag changing) is not a dead socket.
   */
  private serverAnswersPing = false;
  private readonly onWake = (): void => {
    if (typeof document !== 'undefined' && document.visibilityState === 'hidden') return;
    this.retryNow();
  };

  /** `sseUrl` enables the fallback; without it only the WebSocket is used. */
  connect(url: string, token?: string, sseUrl?: string): void {
    this.url = url;
    this.sseUrl = sseUrl ?? '';
    this.token = token ?? '';
    this.reconnectDelay = 1000;
    this.wsFailures = 0;
    if (typeof window !== 'undefined' && typeof window.addEventListener === 'function') {
      window.addEventListener('online', this.onWake);
      if (typeof document !== 'undefined') document.addEventListener('visibilitychange', this.onWake);
    }
    this.doConnect();
  }

  /**
   * The network came back or the tab became visible: a connection waiting out its
   * back-off is tried at once instead of after up to 30 s.
   */
  private retryNow(): void {
    if (!this.url || !this.reconnectTimer) return;
    clearTimeout(this.reconnectTimer);
    this.reconnectTimer = null;
    this.reconnectDelay = 1000;
    if (this.transport === 'sse') {
      if (this.sseUrl && !this.es) this.openEventSource();
    } else {
      this.doConnect();
    }
  }

  private stopKeepalive(): void {
    if (this.keepaliveTimer) clearTimeout(this.keepaliveTimer);
    if (this.staleTimer) clearTimeout(this.staleTimer);
    this.keepaliveTimer = null;
    this.staleTimer = null;
  }

  private scheduleKeepalive(ws: WebSocket): void {
    if (this.keepaliveTimer) clearTimeout(this.keepaliveTimer);
    this.keepaliveTimer = setTimeout(() => {
      this.keepaliveTimer = null;
      if (this.ws !== ws) return;
      this.send({ type: 'Ping' });
      this.scheduleKeepalive(ws);
    }, KEEPALIVE_INTERVAL_MS);
  }

  /** The server spoke on `ws`: push back the deadline after which it counts as dead. */
  private heardFrom(ws: WebSocket): void {
    if (this.staleTimer) clearTimeout(this.staleTimer);
    this.staleTimer = null;
    if (!this.serverAnswersPing) return;
    this.staleTimer = setTimeout(() => {
      this.staleTimer = null;
      if (this.ws !== ws) return;
      console.warn('WS: no traffic for', STALE_AFTER_MS, 'ms, reconnecting');
      // close() on a dead connection can take minutes to report; drop the socket now.
      ws.onclose = null;
      ws.onmessage = null;
      ws.close();
      this.onMainWebSocketClosed(ws, true);
    }, STALE_AFTER_MS);
  }

  getTransport(): Transport {
    return this.transport;
  }

  private setTransport(transport: Transport): void {
    if (this.transport === transport) return;
    this.transport = transport;
    this.transportCallbacks.forEach(cb => cb(transport));
  }

  private doConnect(): void {
    if (this.ws) {
      const old = this.ws;
      this.ws = null;
      old.close();
    }
    this.ws = this.openWebSocket(false);
  }

  /**
   * Open a WebSocket. The main one counts failures and drives reconnects; a probe
   * (opened while on SSE) stays silent until the server speaks, then takes over.
   */
  private openWebSocket(isProbe: boolean): WebSocket {
    const ws = new WebSocket(this.url);
    ws.binaryType = 'arraybuffer';
    // The server greets every connection; hearing anything means the upgrade really worked.
    let heard = false;

    ws.onopen = () => {
      if (this.token) {
        ws.send(encode({ type: 'auth', token: this.token }));
      }
      if (!isProbe) this.connectCallbacks.forEach(cb => cb());
    };

    ws.onmessage = (event) => {
      if (!heard) {
        heard = true;
        if (isProbe) {
          if (this.probe !== ws) return;
          this.promoteProbe(ws);
        } else if (this.ws === ws) {
          this.wsFailures = 0;
          this.reconnectDelay = 1000;
          this.scheduleKeepalive(ws);
        }
      }
      if (this.ws !== ws) return;
      try {
        const msg = decode(event.data as ArrayBuffer) as Record<string, unknown>;
        if (msg.type === 'Pong') this.serverAnswersPing = true;
        this.heardFrom(ws);
        this.handleMessage(msg);
      } catch (e) {
        console.warn('WS: failed to decode message', e);
      }
    };

    ws.onclose = () => {
      if (isProbe) {
        if (this.probe === ws) {
          this.probe = null;
          this.scheduleProbe();
        } else if (this.ws === ws) {
          // The promoted probe is the main socket now.
          this.onMainWebSocketClosed(ws, heard);
        }
        return;
      }
      this.onMainWebSocketClosed(ws, heard);
    };

    ws.onerror = () => {
      // onclose will fire after this
    };
    return ws;
  }

  private onMainWebSocketClosed(ws: WebSocket, heard: boolean): void {
    if (this.ws !== ws) return;
    this.stopKeepalive();
    this.disconnectCallbacks.forEach(cb => cb());
    if (heard) {
      this.wsFailures = 0;
    } else {
      this.wsFailures += 1;
      if (this.sseUrl && this.wsFailures >= WS_FAILURES_BEFORE_FALLBACK && typeof EventSource !== 'undefined') {
        this.ws = null;
        this.startSse();
        return;
      }
    }
    this.scheduleReconnect();
  }

  // ── SSE fallback ─────────────────────────────────────────────

  private startSse(): void {
    this.reconnectDelay = 1000;
    this.setTransport('sse');
    this.openEventSource();
    this.scheduleProbe();
  }

  private openEventSource(): void {
    this.es?.close();
    const es = new EventSource(this.sseUrl, { withCredentials: true });
    this.es = es;
    this.sseUp = false;

    for (const name of SSE_EVENTS) {
      es.addEventListener(name, (event) => {
        if (this.es !== es) return;
        let msg: Record<string, unknown>;
        try {
          msg = JSON.parse((event as MessageEvent).data as string) as Record<string, unknown>;
        } catch (e) {
          console.warn('SSE: failed to decode message', e);
          return;
        }
        if (name === 'AuthOk' && !this.sseUp) {
          // The server's greeting: the stream is live (again).
          this.sseUp = true;
          this.reconnectDelay = 1000;
          this.connectCallbacks.forEach(cb => cb());
        }
        this.handleMessage(msg);
      });
    }

    es.onerror = () => {
      if (this.es !== es) return;
      if (this.sseUp) {
        this.sseUp = false;
        this.disconnectCallbacks.forEach(cb => cb());
      }
      // The browser retries a dropped stream on its own. A refused one (401, a
      // proxy answering with something else) it gives up on: retry it here.
      if (es.readyState === EventSource.CLOSED) {
        es.close();
        this.es = null;
        this.reconnectTimer = setTimeout(() => {
          this.reconnectTimer = null;
          if (this.sseUrl && this.transport === 'sse') this.openEventSource();
        }, jittered(this.reconnectDelay));
        this.reconnectDelay = Math.min(this.reconnectDelay * 2, this.maxReconnectDelay);
      }
    };
  }

  private closeEventSource(): void {
    this.es?.close();
    this.es = null;
    this.sseUp = false;
  }

  private scheduleProbe(): void {
    if (this.probeTimer || this.transport !== 'sse' || !this.url) return;
    this.probeTimer = setTimeout(() => {
      this.probeTimer = null;
      if (this.transport !== 'sse' || !this.url) return;
      this.probe = this.openWebSocket(true);
    }, SSE_PROBE_INTERVAL_MS);
  }

  /** A probe WebSocket heard the server: it carries the data from now on. */
  private promoteProbe(ws: WebSocket): void {
    this.probe = null;
    this.closeEventSource();
    if (this.reconnectTimer) {
      clearTimeout(this.reconnectTimer);
      this.reconnectTimer = null;
    }
    this.ws = ws;
    this.wsFailures = 0;
    this.reconnectDelay = 1000;
    this.setTransport('ws');
    this.scheduleKeepalive(ws);
    this.connectCallbacks.forEach(cb => cb());
  }

  private handleMessage(msg: Record<string, unknown>): void {
    switch (msg.type) {
      case 'TagUpdateBatch': {
        const batch = (msg as unknown as TagUpdateBatch).updates;
        if (batch.length > 0) {
          for (const u of batch) this.tagUpdateCallbacks.forEach(cb => cb(u));
          this.tagBatchCallbacks.forEach(cb => cb(batch));
        }
        break;
      }
      case 'TagUpdate': {
        const update = msg as unknown as TagUpdate;
        this.tagUpdateCallbacks.forEach(cb => cb(update));
        this.batchBuffer.push(update);
        if (!this.batchTimer) {
          this.batchTimer = setTimeout(() => {
            const batch = this.batchBuffer;
            this.batchBuffer = [];
            this.batchTimer = null;
            if (batch.length > 0) {
              this.tagBatchCallbacks.forEach(cb => cb(batch));
            }
          }, 200);
        }
        break;
      }
      case 'CommandAck':
        this.commandAckCallbacks.forEach(cb => cb(msg as unknown as CommandAck));
        break;
      case 'Alarm':
        this.alarmCallbacks.forEach(cb => cb(msg as unknown as AlarmEvent));
        break;
      case 'AlarmUpdate':
        this.alarmUpdateCallbacks.forEach(cb => cb(msg as unknown as AlarmUpdateEvent));
        break;
      case 'DeviceStatus':
        this.deviceStatusCallbacks.forEach(cb => cb(msg as unknown as DeviceStatus));
        break;
      default:
        if (typeof msg.type === 'string' && msg.type.startsWith('Design')) {
          this.designCallbacks.forEach(cb => cb(msg as unknown as DesignServerMessage));
        }
    }
  }

  subscribe(tagIds: string[]): void {
    this.send({ type: 'Subscribe', tag_ids: tagIds });
  }

  unsubscribe(tagIds: string[]): void {
    this.send({ type: 'Unsubscribe', tag_ids: tagIds });
  }

  writeTag(tagId: string, value: number, reason?: string): string {
    const commandId = `ws-${Date.now()}-${Math.random().toString(36).slice(2, 8)}`;
    this.send({ type: 'TagWrite', tag_id: tagId, value, command_id: commandId, reason });
    return commandId;
  }

  disconnect(): void {
    if (this.reconnectTimer) {
      clearTimeout(this.reconnectTimer);
      this.reconnectTimer = null;
    }
    if (this.probeTimer) {
      clearTimeout(this.probeTimer);
      this.probeTimer = null;
    }
    if (this.batchTimer) {
      clearTimeout(this.batchTimer);
      this.batchTimer = null;
    }
    this.stopKeepalive();
    this.serverAnswersPing = false;
    if (typeof window !== 'undefined' && typeof window.removeEventListener === 'function') {
      window.removeEventListener('online', this.onWake);
      if (typeof document !== 'undefined') document.removeEventListener('visibilitychange', this.onWake);
    }
    // Clear URLs before closing to prevent race with onclose triggering reconnect
    this.url = '';
    this.sseUrl = '';
    this.token = '';
    this.closeEventSource();
    this.probe?.close();
    this.probe = null;
    this.ws?.close();
    this.ws = null;
    this.transport = 'ws';
    // Clear all callback arrays to prevent accumulation across reconnect cycles
    this.tagUpdateCallbacks = [];
    this.tagBatchCallbacks = [];
    this.commandAckCallbacks = [];
    this.alarmCallbacks = [];
    this.alarmUpdateCallbacks = [];
    this.deviceStatusCallbacks = [];
    this.disconnectCallbacks = [];
    this.transportCallbacks = [];
    this.connectCallbacks = [];
    this.designCallbacks = [];
  }

  isConnected(): boolean {
    if (this.transport === 'sse') return this.sseUp;
    return this.ws?.readyState === WebSocket.OPEN;
  }

  /** Whether tag writes can go over this connection. The SSE stream is read-only. */
  canWrite(): boolean {
    return this.transport === 'ws' && this.ws?.readyState === WebSocket.OPEN;
  }

  onTagUpdate(cb: TagUpdateCallback): void { addUnique(this.tagUpdateCallbacks, cb); }
  offTagUpdate(cb: TagUpdateCallback): void { this.tagUpdateCallbacks = this.tagUpdateCallbacks.filter(c => c !== cb); }
  onTagBatch(cb: TagBatchCallback): void { addUnique(this.tagBatchCallbacks, cb); }
  offTagBatch(cb: TagBatchCallback): void { this.tagBatchCallbacks = this.tagBatchCallbacks.filter(c => c !== cb); }
  onCommandAck(cb: CommandAckCallback): void { addUnique(this.commandAckCallbacks, cb); }
  offCommandAck(cb: CommandAckCallback): void { this.commandAckCallbacks = this.commandAckCallbacks.filter(c => c !== cb); }
  onAlarm(cb: AlarmCallback): void { addUnique(this.alarmCallbacks, cb); }
  onAlarmUpdate(cb: AlarmUpdateCallback): void { addUnique(this.alarmUpdateCallbacks, cb); }
  onDeviceStatus(cb: DeviceStatusCallback): void { addUnique(this.deviceStatusCallbacks, cb); }
  onDisconnect(cb: DisconnectCallback): void { addUnique(this.disconnectCallbacks, cb); }
  onConnect(cb: () => void): void { addUnique(this.connectCallbacks, cb); }
  offConnect(cb: () => void): void { this.connectCallbacks = this.connectCallbacks.filter(c => c !== cb); }
  offDisconnect(cb: DisconnectCallback): void { this.disconnectCallbacks = this.disconnectCallbacks.filter(c => c !== cb); }
  onDesign(cb: DesignCallback): void { addUnique(this.designCallbacks, cb); }
  offDesign(cb: DesignCallback): void { this.designCallbacks = this.designCallbacks.filter(c => c !== cb); }

  /** Send a collaboration frame. False when the WebSocket is not open (the SSE fallback cannot carry them). */
  sendDesign(msg: DesignClientMessage): boolean {
    if (this.transport !== 'ws' || this.ws?.readyState !== WebSocket.OPEN) return false;
    this.ws.send(encode(msg));
    return true;
  }
  /** Called when the live data switches between the WebSocket and the SSE fallback. */
  onTransportChange(cb: TransportCallback): void { addUnique(this.transportCallbacks, cb); }

  private send(msg: Record<string, unknown>): void {
    if (this.ws?.readyState === WebSocket.OPEN) {
      this.ws.send(encode(msg));
    }
  }

  private scheduleReconnect(): void {
    if (!this.url || this.reconnectTimer) return;
    const delay = this.reconnectDelay;
    this.reconnectDelay = Math.min(this.reconnectDelay * 2, this.maxReconnectDelay);
    this.reconnectTimer = setTimeout(() => {
      this.reconnectTimer = null;
      if (this.url) this.doConnect();
    }, jittered(delay));
  }
}

// ── Singleton for use outside React tree ──

let _instance: VoltrusWebSocket | null = null;
type ReadyListener = (ws: VoltrusWebSocket) => void;
const _readyListeners = new Set<ReadyListener>();

function _notifyReady(ws: VoltrusWebSocket): void {
  _readyListeners.forEach(fn => fn(ws));
}

export function setWsInstance(ws: VoltrusWebSocket | null): void {
  _instance = ws;
  if (ws) {
    if (ws.isConnected()) _notifyReady(ws);
    ws.onConnect(() => _notifyReady(ws));
  }
}

/**
 * Register a callback that fires as soon as a WS instance is connected.
 * If already connected, fires synchronously. Returns an unsubscribe function.
 */
export function onWsReady(fn: ReadyListener): () => void {
  _readyListeners.add(fn);
  if (_instance?.isConnected()) fn(_instance);
  return () => { _readyListeners.delete(fn); };
}

export function getWsInstance(): VoltrusWebSocket | null {
  return _instance;
}

/** How long a WebSocket write waits for the device before it is reported as queued. */
export const WS_WRITE_WAIT_MS = 15_000;

/**
 * Write a tag over the WebSocket and wait for the outcome. The server answers a refused
 * write at once (`rejected`) and an accepted one when the command is final (`acked`,
 * `failed`, `expired`); until then it is queued, so a device that does not answer within
 * `waitMs` is reported as `queued`, not as a failure. Resolves `null` when there is no
 * WebSocket that can write, or when the server wants the write signed: the caller then
 * uses the REST endpoint, whose fetch prompts for the signature. Rejects with the server's
 * reason when the write was refused or failed.
 */
export function wsWriteTagResult(
  sensorId: number,
  value: number,
  waitMs: number = WS_WRITE_WAIT_MS,
): Promise<'acked' | 'queued' | null> {
  const ws = _instance;
  if (!ws || !ws.canWrite()) return Promise.resolve(null);
  return new Promise((resolve, reject) => {
    let commandId = '';
    const finish = (fn: () => void) => {
      clearTimeout(timer);
      ws.offCommandAck(onAck);
      fn();
    };
    const onAck = (ack: CommandAck) => {
      if (ack.command_id !== commandId) return;
      if (ack.signature_required || ack.status === 'signature_required') finish(() => resolve(null));
      else if (ack.status === 'acked') finish(() => resolve('acked'));
      else if (['rejected', 'failed', 'expired', 'cancelled'].includes(ack.status)) {
        finish(() => reject(new Error(ack.device_response || `Write ${ack.status}`)));
      }
      // sent, queued: the command is on its way.
    };
    const timer = setTimeout(() => finish(() => resolve('queued')), waitMs);
    // Listen before sending: a refusal can arrive as soon as the frame is out.
    ws.onCommandAck(onAck);
    commandId = ws.writeTag(String(sensorId), value);
  });
}

export function isWsConnected(): boolean {
  return _instance?.isConnected() ?? false
}
