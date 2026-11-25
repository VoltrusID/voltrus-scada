import { afterEach, describe, expect, it, vi } from 'vitest'
import { render, screen, fireEvent } from '@testing-library/react'
import { EditPanel } from '../../src/EditPanel'
import type { SensorConfig } from '../../src/types'

const tag: SensorConfig = {
  id: 7, key: 'tank.level', name: 'Tank level', unit: '%', color: '#00bcd4',
  min_val: 0, max_val: 100, warn_val: 80, crit_val: 90, base_val: 50, sort_order: 0, decimals: 1,
  source_type: 'simulated', source_config: '{}', device_id: '', equipment_id: null,
  chart_type: 'sparkline', path: null, on_delay_secs: 0, off_delay_secs: 0,
}
const suggestion = { on_delay_secs: 5, deadband: null, reason: 'Noisy signal.', chattering: false, max_alarms_per_hour: 3, alarms_last_hour: 1 }

function open() {
  vi.stubGlobal('fetch', vi.fn(async () => new Response(JSON.stringify(suggestion), { status: 200 })))
  const onSave = vi.fn()
  render(<EditPanel sensor={tag} activeScreen="main" onSave={onSave} onDelete={() => {}} onClose={() => {}} shortcuts={{}} />)
  return onSave
}

describe('suggested alarm on-delay', () => {
  afterEach(() => { vi.unstubAllGlobals() })

  it('opening and saving without accepting keeps the stored delay', async () => {
    const onSave = open()
    await screen.findByRole('button', { name: 'Use 5 s' })
    fireEvent.click(screen.getByRole('button', { name: 'Save' }))
    expect(onSave).toHaveBeenCalledTimes(1)
    expect(onSave.mock.calls[0][0].on_delay_secs).toBe(0)
  })

  it('"Use N s" puts the suggestion in the saved tag', async () => {
    const onSave = open()
    fireEvent.click(await screen.findByRole('button', { name: 'Use 5 s' }))
    expect(screen.queryByRole('button', { name: 'Use 5 s' })).toBeNull()
    fireEvent.click(screen.getByRole('button', { name: 'Save' }))
    expect(onSave.mock.calls[0][0].on_delay_secs).toBe(5)
  })
})
