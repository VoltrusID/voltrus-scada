import { afterEach, describe, expect, it, vi } from 'vitest'
import { render, act } from '@testing-library/react'
import { encode } from '@msgpack/msgpack'
import { VoltrusWebSocket, setWsInstance } from '../../src/api/websocket'
import { createDashboardStore, DashboardStoreProvider } from '../../src/store/dashboardStore'
import { useWSData } from '../../src/hooks/useWSData'

class FakeWebSocket {
  static last: FakeWebSocket
  static OPEN = 1
  readyState = 0
  binaryType = ''
  onopen: (() => void) | null = null
  onmessage: ((e: { data: ArrayBuffer }) => void) | null = null
  onclose: (() => void) | null = null
  onerror: (() => void) | null = null
  constructor(public url: string) { FakeWebSocket.last = this }
  send() {}
  close() { this.readyState = 3; this.onclose?.() }
  hear(msg: Record<string, unknown>) {
    const b = encode(msg)
    this.onmessage?.({ data: b.buffer.slice(b.byteOffset, b.byteOffset + b.byteLength) as ArrayBuffer })
  }
}

function Probe() { useWSData(true, []); return null }

describe('live status in the footer', () => {
  afterEach(() => { vi.unstubAllGlobals(); setWsInstance(null) })

  it('goes back to not connected when the socket drops, and live again on reconnect', () => {
    vi.stubGlobal('WebSocket', FakeWebSocket)
    vi.useFakeTimers()
    const store = createDashboardStore()
    render(<DashboardStoreProvider value={store}><Probe /></DashboardStoreProvider>)
    const ws = new VoltrusWebSocket()
    ws.connect('ws://x/api/v1/ws')
    setWsInstance(ws)
    act(() => { FakeWebSocket.last.readyState = 1; FakeWebSocket.last.onopen?.(); FakeWebSocket.last.hear({ type: 'AuthOk' }) })
    expect(store.getState().wsConnected).toBe(true)

    act(() => { FakeWebSocket.last.close() })
    expect(store.getState().wsConnected).toBe(false)

    act(() => { vi.advanceTimersByTime(2000) })
    act(() => { FakeWebSocket.last.readyState = 1; FakeWebSocket.last.onopen?.(); FakeWebSocket.last.hear({ type: 'AuthOk' }) })
    expect(store.getState().wsConnected).toBe(true)
    ws.disconnect()
    vi.useRealTimers()
  })
})
