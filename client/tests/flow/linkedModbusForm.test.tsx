import { afterEach, describe, expect, it, vi } from 'vitest'
import { render, screen, fireEvent, waitFor } from '@testing-library/react'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { ModbusDeviceForm } from '../../src/ModbusDeviceForm'

const register = { id: 1, device_id: 'm', name: 'voltage', address: 0, register_type: 'float32', register_area: 'holding', scale: 1, sort_order: 0, zero_based: true }
const device = {
  id: 'm', name: 'Meter', ip: '127.0.0.1', port: 502, unit_id: 1, interval_ms: 1000, endianness: 'big',
  enabled: true, created_at: 0, updated_at: 0, register_count: 1,
  template: { template_id: 'acme-meter', version: '1.0.0', source: 'local' },
  registers: [register],
}

function open(onUnlink?: (id: string) => Promise<boolean>) {
  vi.stubGlobal('fetch', vi.fn(async () => new Response('{}', { status: 200 })))
  const onSave = vi.fn()
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  render(<QueryClientProvider client={qc}><ModbusDeviceForm device={device} onSave={onSave} onClose={() => {}} onUnlink={onUnlink} /></QueryClientProvider>)
  return onSave
}

describe('editing a Modbus device linked to a template', () => {
  afterEach(() => { vi.unstubAllGlobals() })

  it('sends the device without its registers, which are read-only', () => {
    const onSave = open()
    expect(screen.getByText(/follow template acme-meter 1.0.0/)).toBeTruthy()
    expect((screen.getByDisplayValue('voltage') as HTMLInputElement).closest('fieldset')!.disabled).toBe(true)
    expect(screen.queryByRole('button', { name: '+ Add' })).toBeNull()
    fireEvent.change(screen.getByDisplayValue('127.0.0.1'), { target: { value: '10.0.0.9' } })
    fireEvent.click(screen.getByRole('button', { name: 'Update' }))
    expect(onSave).toHaveBeenCalledTimes(1)
    expect(onSave.mock.calls[0][0].ip).toBe('10.0.0.9')
    expect('registers' in onSave.mock.calls[0][0]).toBe(false)
  })

  it('unlinking makes the registers editable and sends them', async () => {
    const onSave = open(async () => true)
    fireEvent.click(screen.getByRole('button', { name: 'Unlink' }))
    await waitFor(() => expect(screen.queryByRole('button', { name: '+ Add' })).toBeTruthy())
    fireEvent.click(screen.getByRole('button', { name: 'Update' }))
    expect(onSave.mock.calls[0][0].registers).toHaveLength(1)
  })
})
