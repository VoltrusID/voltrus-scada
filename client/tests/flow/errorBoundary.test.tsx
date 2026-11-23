import { afterEach, describe, expect, it, vi } from 'vitest'
import { lazy, Suspense, useState } from 'react'
import { render, screen, fireEvent, waitFor } from '@testing-library/react'
import { ErrorBoundary } from '../../src/components/ErrorBoundary'

describe('the app error boundary', () => {
  afterEach(() => { sessionStorage.clear(); vi.restoreAllMocks() })

  it('offers a retry for a render error and recovers', () => {
    vi.spyOn(console, 'error').mockImplementation(() => {})
    let fail = true
    function Flaky() { if (fail) throw new Error('boom'); return <div>page</div> }
    render(<ErrorBoundary><Flaky /></ErrorBoundary>)
    expect(screen.getByRole('alert').textContent).toContain('Something went wrong')
    fail = false
    fireEvent.click(screen.getByRole('button', { name: 'Try again' }))
    expect(screen.getByText('page')).toBeTruthy()
  })

  it('a missing chunk with the server down keeps the in-app screen and reloads once it is back', async () => {
    vi.spyOn(console, 'error').mockImplementation(() => {})
    let up = false
    vi.stubGlobal('fetch', vi.fn(async () => { if (!up) throw new TypeError('Failed to fetch'); return new Response(null, { status: 200 }) }))
    const Page = lazy(() => Promise.reject(new TypeError('Failed to fetch dynamically imported module: /assets/Page.js')))
    render(<ErrorBoundary><Suspense fallback={null}><Page /></Suspense></ErrorBoundary>)
    await waitFor(() => expect(screen.getByRole('status').textContent).toContain('not reachable'))
    expect(screen.getByRole('button', { name: 'Reload' })).toBeTruthy()
    expect(sessionStorage.getItem('voltrus:chunk-reload-at')).toBeNull()
    up = true
    window.dispatchEvent(new Event('online'))
    await waitFor(() => expect(sessionStorage.getItem('voltrus:chunk-reload-at')).not.toBeNull())
    vi.unstubAllGlobals()
  })

  it('clears the error when the reset key changes', () => {
    vi.spyOn(console, 'error').mockImplementation(() => {})
    function Boom(): null { throw new Error('boom') }
    function Host() {
      const [route, setRoute] = useState('/a')
      return (
        <>
          <button onClick={() => setRoute('/b')}>go</button>
          <ErrorBoundary resetKey={route}>{route === '/a' ? <Boom /> : <div>other page</div>}</ErrorBoundary>
        </>
      )
    }
    render(<Host />)
    expect(screen.getByRole('alert')).toBeTruthy()
    fireEvent.click(screen.getByText('go'))
    expect(screen.getByText('other page')).toBeTruthy()
  })
})
