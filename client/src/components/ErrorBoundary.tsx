import { Component, type ErrorInfo, type ReactNode } from 'react'
import i18n from 'i18next'
import { isChunkLoadError, reloadOnceForNewBuild, CHUNK_RELOAD_GUARD_MS } from '../chunkReload'

interface Props {
  children: ReactNode
  /** Changing this clears a shown error, e.g. the route, so navigating away recovers. */
  resetKey?: unknown
  /** Fill the viewport (app root) instead of the surrounding box. */
  fullScreen?: boolean
}

interface State {
  error: Error | null
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

  static getDerivedStateFromError(error: Error): State {
    return { error }
  }

  componentDidCatch(error: Error, info: ErrorInfo) {
    console.error('UI error:', error, info.componentStack)
    if (isChunkLoadError(error)) {
      if (reloadOnceForNewBuild()) return
      this.autoRetry = setTimeout(() => reloadOnceForNewBuild(), CHUNK_RELOAD_GUARD_MS)
    }
  }

  componentDidUpdate(prev: Props) {
    if (this.state.error && prev.resetKey !== this.props.resetKey) this.reset()
  }

  componentWillUnmount() {
    if (this.autoRetry) clearTimeout(this.autoRetry)
  }

  private reset = () => {
    if (this.autoRetry) clearTimeout(this.autoRetry)
    this.autoRetry = null
    this.setState({ error: null })
  }

  private retry = () => {
    // A missing chunk only comes back with a fresh page.
    if (this.state.error && isChunkLoadError(this.state.error)) window.location.reload()
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
