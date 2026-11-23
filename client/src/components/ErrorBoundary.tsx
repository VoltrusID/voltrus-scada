import { Component, type ErrorInfo, type ReactNode } from 'react'
import i18n from 'i18next'
import { isChunkLoadError, reloadIfServerUp, recoveryDelayMs, serverReachable } from '../chunkReload'

interface Props {
  children: ReactNode
  /** Changing this clears a shown error, e.g. the route, so navigating away recovers. */
  resetKey?: unknown
  /** Fill the viewport (app root) instead of the surrounding box. */
  fullScreen?: boolean
}

interface State {
  error: Error | null
  /** The last probe found the server unreachable. */
  serverDown?: boolean
}

/**
 * Catches render errors and failed page loads. A missing build chunk (the
 * server was updated) reloads the page once; otherwise the user gets a retry.
 * An unattended kiosk retries a missing chunk by itself once the reload guard
 * has passed.
 */
export class ErrorBoundary extends Component<Props, State> {
  state: State = { error: null }
  private autoRetry: ReturnType<typeof setTimeout> | null = null
  private attempt = 0
  private readonly onOnline = () => { this.attempt = 0; this.recover() }

  static getDerivedStateFromError(error: Error): State {
    return { error }
  }

  componentDidCatch(error: Error, info: ErrorInfo) {
    console.error('UI error:', error, info.componentStack)
    if (isChunkLoadError(error)) {
      window.addEventListener('online', this.onOnline)
      this.recover()
    }
  }

  /**
   * A missing chunk: reload once the server answers. While it is down the
   * in-app screen stays (a reload would leave the browser's error page, which
   * never comes back) and the probe is repeated with back-off and on `online`.
   */
  private recover = () => {
    if (this.autoRetry) clearTimeout(this.autoRetry)
    this.autoRetry = null
    void reloadIfServerUp().then(outcome => {
      if (outcome === 'reloaded' || !this.state.error) return
      this.setState({ serverDown: outcome === 'server-down' })
      const delay = recoveryDelayMs(this.attempt++)
      this.autoRetry = setTimeout(this.recover, delay)
    })
  }

  private stopRecovery() {
    if (this.autoRetry) clearTimeout(this.autoRetry)
    this.autoRetry = null
    this.attempt = 0
    window.removeEventListener('online', this.onOnline)
  }

  componentDidUpdate(prev: Props) {
    if (this.state.error && prev.resetKey !== this.props.resetKey) this.reset()
  }

  componentWillUnmount() {
    this.stopRecovery()
  }

  private reset = () => {
    this.stopRecovery()
    this.setState({ error: null, serverDown: false })
  }

  private retry = () => {
    // A missing chunk only comes back with a fresh page, and only from a server that answers.
    if (this.state.error && isChunkLoadError(this.state.error)) {
      // A deliberate click skips the loop guard, but still not onto a dead server.
      void serverReachable().then(up => {
        if (up) window.location.reload()
        else { this.setState({ serverDown: true }); this.attempt = 0; this.recover() }
      })
    }
    else this.reset()
  }

  render() {
    const { error } = this.state
    if (!error) return this.props.children
    const chunk = isChunkLoadError(error)
    // The default text covers an error before the translations have loaded.
    const t = (key: string, defaultValue: string) => i18n.t(key, { defaultValue })
    return (
      <div role="alert" style={{
        display: 'flex', flexDirection: 'column', alignItems: 'center', justifyContent: 'center', gap: 12,
        padding: 32, textAlign: 'center', color: 'var(--fg)', fontFamily: 'var(--font-ui)',
        ...(this.props.fullScreen ? { minHeight: '100dvh', background: 'var(--bg)' } : { flex: 1, minHeight: 240 }),
      }}>
        <div style={{ fontSize: 16, fontWeight: 600 }}>
          {chunk ? t('errors.pageNotLoaded', 'This page could not be loaded') : t('errors.pageCrashed', 'Something went wrong on this page')}
        </div>
        <div style={{ fontSize: 13, color: 'var(--muted)', maxWidth: 420 }}>
          {chunk
            ? t('errors.pageNotLoadedDesc', 'The server may have been updated or is not reachable. Reloading fetches the current version.')
            : t('errors.pageCrashedDesc', 'The rest of the app keeps running. Try again, or go back to the dashboard.')}
        </div>
        {chunk && this.state.serverDown && (
          <div role="status" style={{ fontSize: 13, color: 'var(--warn)' }}>
            {t('errors.serverDown', 'The server is not reachable. Retrying automatically.')}
          </div>
        )}
        <button onClick={this.retry} style={{
          padding: '8px 18px', borderRadius: 6, border: '1px solid var(--cyan)', background: 'transparent',
          color: 'var(--cyan)', fontSize: 13, fontWeight: 600, cursor: 'pointer', minHeight: 36,
        }}>
          {chunk ? t('errors.reload', 'Reload') : t('errors.tryAgain', 'Try again')}
        </button>
      </div>
    )
  }
}
