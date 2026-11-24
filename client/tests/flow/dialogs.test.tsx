import { afterEach, describe, expect, it, vi } from 'vitest'
import { useState } from 'react'
import { render, screen, fireEvent, act } from '@testing-library/react'
import { ConfirmDialog, PromptDialog } from '../../src/ScreenDialog'
import { SignatureDialog } from '../../src/signature/SignatureDialog'
import { requestSignature, cancelAllSignaturePrompts } from '../../src/signature/prompt'
import { TabList } from '../../src/components/TabList'

function Opener({ dialog }: { dialog: (close: () => void) => React.ReactNode }) {
  const [open, setOpen] = useState(false)
  return (
    <>
      <button onClick={() => setOpen(true)}>open</button>
      {open && dialog(() => setOpen(false))}
    </>
  )
}

describe('dialogs', () => {
  afterEach(() => { cancelAllSignaturePrompts() })

  it('are named modal dialogs that close on Escape and give focus back', () => {
    render(<Opener dialog={close => <ConfirmDialog title="Delete Screen" message="Sure?" onConfirm={() => {}} onClose={close} />} />)
    const opener = screen.getByText('open')
    opener.focus()
    fireEvent.click(opener)
    const dialog = screen.getByRole('dialog', { name: 'Delete Screen' })
    expect(dialog.getAttribute('aria-modal')).toBe('true')
    expect(dialog.contains(document.activeElement)).toBe(true)
    fireEvent.keyDown(document.activeElement!, { key: 'Escape' })
    expect(screen.queryByRole('dialog')).toBeNull()
    expect(document.activeElement).toBe(opener)
  })

  it('keep Tab inside', () => {
    render(<ConfirmDialog title="T" message="m" onConfirm={() => {}} onClose={() => {}} />)
    const [cancel, del] = screen.getAllByRole('button')
    del.focus()
    fireEvent.keyDown(del, { key: 'Tab' })
    expect(document.activeElement).toBe(cancel)
    fireEvent.keyDown(cancel, { key: 'Tab', shiftKey: true })
    expect(document.activeElement).toBe(del)
  })

  it('the prompt dialog does not close on Escape while recording a shortcut', () => {
    const onClose = vi.fn()
    render(<PromptDialog title="New Screen" label="Screen name" shortcutValue="" onSubmit={() => {}} onClose={onClose} />)
    fireEvent.click(screen.getByText('Record'))
    fireEvent.keyDown(screen.getByRole('dialog'), { key: 'Escape' })
    expect(onClose).not.toHaveBeenCalled()
  })
})

describe('the signature prompt', () => {
  it('says who is signing, traps focus and hands it back', async () => {
    const opener = document.createElement('button')
    document.body.appendChild(opener)
    opener.focus()
    render(<SignatureDialog username="alice" />)
    let answer: Promise<unknown> = Promise.resolve()
    act(() => { answer = requestSignature({ action: 'tag.write', meaning: 'Write 5 to pump speed' }) })
    const dialog = screen.getByRole('dialog', { name: 'Electronic signature required' })
    expect(dialog.textContent).toContain('Signing as alice')
    expect(dialog.contains(document.activeElement)).toBe(true)
    act(() => { fireEvent.keyDown(document.activeElement!, { key: 'Escape' }) })
    expect(await answer).toBeNull()
    expect(screen.queryByRole('dialog')).toBeNull()
    expect(document.activeElement).toBe(opener)
    opener.remove()
  })
})

describe('tabs', () => {
  it('are buttons with one tab stop and arrow-key movement', () => {
    function Host() {
      const [tab, setTab] = useState<'a' | 'b' | 'c'>('a')
      return <TabList label="Sections" idPrefix="t" value={tab} onChange={setTab} tabs={[{ id: 'a', label: 'A' }, { id: 'b', label: 'B', disabled: true }, { id: 'c', label: 'C' }]} />
    }
    render(<Host />)
    const tabs = screen.getAllByRole('tab')
    expect(tabs.map(t => t.tabIndex)).toEqual([0, -1, -1])
    tabs[0].focus()
    fireEvent.keyDown(tabs[0], { key: 'ArrowRight' })
    expect(screen.getByRole('tab', { name: 'C' }).getAttribute('aria-selected')).toBe('true')
    expect(document.activeElement).toBe(screen.getByRole('tab', { name: 'C' }))
    fireEvent.keyDown(document.activeElement!, { key: 'Home' })
    expect(screen.getByRole('tab', { name: 'A' }).getAttribute('aria-selected')).toBe('true')
  })
})

import WidgetShell from '../../src/scada2/widgets/WidgetShell'

describe('SCADA widgets in run mode', () => {
  const instance = { id: 'w1', type: 'pump', sensorId: 3, x: 0, y: 0, width: 60, height: 60, config: {} }
  const base = { instance, sensor: undefined, data: undefined, sensors: [], currentValues: {}, editMode: false, selected: false, scale: 1, onMove: () => {}, onResize: () => {} }

  it('a widget with a faceplate opens with Enter or Space', () => {
    const onSelect = vi.fn()
    render(<WidgetShell {...base} onSelect={onSelect} activatable />)
    const btn = screen.getByRole('button', { name: /open faceplate/ })
    fireEvent.keyDown(btn, { key: 'Enter' })
    fireEvent.keyDown(btn, { key: ' ' })
    expect(onSelect).toHaveBeenCalledTimes(2)
  })

  it('other widgets are not keyboard buttons', () => {
    render(<WidgetShell {...base} onSelect={() => {}} />)
    expect(screen.queryByRole('button', { name: /open faceplate/ })).toBeNull()
  })
})
