import { encode, decode } from '@msgpack/msgpack';

export interface TagUpdate {
  tag_id: string;
  value: number;
  quality: string;
  ts: number;
  simulated: boolean;
}

export interface TagUpdateBatch {
  updates: TagUpdate[];
}

export interface CommandAck {
  command_id: string;
  status: string;
  device_response?: string;
  latency_ms?: number;
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

type TagUpdateCallback = (update: TagUpdate) => void;
type TagBatchCallback = (updates: TagUpdate[]) => void;
type CommandAckCallback = (ack: CommandAck) => void;
type AlarmCallback = (alarm: AlarmEvent) => void;
type DeviceStatusCallback = (status: DeviceStatus) => void;
type DisconnectCallback = () => void;

export class VoltrusWebSocket {
  private ws: WebSocket | null = null;
  private reconnectTimer: ReturnType<typeof setTimeout> | null = null;
  private reconnectDelay = 1000;
  private maxReconnectDelay = 30000;
  private tagUpdateCallbacks: TagUpdateCallback[] = [];
  private tagBatchCallbacks: TagBatchCallback[] = [];
  private commandAckCallbacks: CommandAckCallback[] = [];
  private alarmCallbacks: AlarmCallback[] = [];
  private deviceStatusCallbacks: DeviceStatusCallback[] = [];
  private disconnectCallbacks: DisconnectCallback[] = [];
  private url = '';
  private token = '';
  private batchBuffer: TagUpdate[] = [];
  private batchTimer: ReturnType<typeof setTimeout> | null = null;
  private connectCallbacks: (() => void)[] = [];

  connect(url: string, token?: string): void {
    this.url = url;
    this.token = token ?? '';
    this.reconnectDelay = 1000;
    this.doConnect();
  }

  private doConnect(): void {
    if (this.ws) {
      this.ws.close();
    }
    this.ws = new WebSocket(this.url);
    this.ws.binaryType = 'arraybuffer';

    this.ws.onopen = () => {
      this.reconnectDelay = 1000;
      if (this.token) {
        const authMsg = encode({ type: 'auth', token: this.token });
        this.ws?.send(authMsg);
      }
      this.connectCallbacks.forEach(cb => cb());
    };

    this.ws.onmessage = (event) => {
      const data = event.data as ArrayBuffer;
      try {
        const msg = decode(data) as Record<string, unknown>;
        this.handleMessage(msg);
      } catch (e) {
        console.warn('WS: failed to decode message', e);
      }
    };

    this.ws.onclose = () => {
      this.disconnectCallbacks.forEach(cb => cb());
      this.scheduleReconnect();
    };

    this.ws.onerror = () => {
      // onclose will fire after this
    };
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
      case 'DeviceStatus':
        this.deviceStatusCallbacks.forEach(cb => cb(msg as unknown as DeviceStatus));
        break;
    }
  }

  subscribe(tagIds: string[]): void {
    this.send({ type: 'Subscribe', tag_ids: tagIds });
  }

  unsubscribe(tagIds: string[]): void {
    this.send({ type: 'Unsubscribe', tag_ids: tagIds });
  }

  writeTag(tagId: string, value: number): string {
    const commandId = `ws-${Date.now()}-${Math.random().toString(36).slice(2, 8)}`;
    this.send({ type: 'TagWrite', tag_id: tagId, value, command_id: commandId });
    return commandId;
  }

  disconnect(): void {
    if (this.reconnectTimer) {
      clearTimeout(this.reconnectTimer);
      this.reconnectTimer = null;
    }
    // Clear URL before closing to prevent race with onclose triggering reconnect
    this.url = '';
    this.token = '';
    this.ws?.close();
    this.ws = null;
    // Clear all callback arrays to prevent accumulation across reconnect cycles
    this.tagUpdateCallbacks = [];
    this.tagBatchCallbacks = [];
    this.commandAckCallbacks = [];
    this.alarmCallbacks = [];
    this.deviceStatusCallbacks = [];
    this.disconnectCallbacks = [];
    this.connectCallbacks = [];
  }

  isConnected(): boolean {
    return this.ws?.readyState === WebSocket.OPEN;
  }

  onTagUpdate(cb: TagUpdateCallback): void { this.tagUpdateCallbacks.push(cb); }
  offTagUpdate(cb: TagUpdateCallback): void { this.tagUpdateCallbacks = this.tagUpdateCallbacks.filter(c => c !== cb); }
  onTagBatch(cb: TagBatchCallback): void { this.tagBatchCallbacks.push(cb); }
  offTagBatch(cb: TagBatchCallback): void { this.tagBatchCallbacks = this.tagBatchCallbacks.filter(c => c !== cb); }
  onCommandAck(cb: CommandAckCallback): void { this.commandAckCallbacks.push(cb); }
  onAlarm(cb: AlarmCallback): void { this.alarmCallbacks.push(cb); }
  onDeviceStatus(cb: DeviceStatusCallback): void { this.deviceStatusCallbacks.push(cb); }
  onDisconnect(cb: DisconnectCallback): void { this.disconnectCallbacks.push(cb); }
  onConnect(cb: () => void): void { this.connectCallbacks.push(cb); }

  private send(msg: Record<string, unknown>): void {
    if (this.ws?.readyState === WebSocket.OPEN) {
      this.ws.send(encode(msg));
    }
  }

  private scheduleReconnect(): void {
    if (!this.url) return;
    this.reconnectTimer = setTimeout(() => {
      this.reconnectDelay = Math.min(this.reconnectDelay * 2, this.maxReconnectDelay);
      this.doConnect();
    }, this.reconnectDelay);
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

export function wsWriteTag(sensorId: number, value: number): boolean {
  if (_instance && _instance.isConnected()) {
    _instance.writeTag(String(sensorId), value);
    return true;
  }
  return false;
}

export function isWsConnected(): boolean {
  return _instance?.isConnected() ?? false
}
