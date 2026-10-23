import {
  usePushSupported,
  usePushPermission,
  useVapidKey,
  useSubscribePush,
  useUnsubscribePush,
} from './queries/push'
import { btnStyle } from './styles'

export function PushSettings() {
  const supported = usePushSupported()
  const { data: permission } = usePushPermission()
  const { data: vapidData, isLoading: loadingKey } = useVapidKey()
  const subscribeMut = useSubscribePush()
  const unsubscribeMut = useUnsubscribePush()

  if (!supported) {
    return (
      <div style={{
        padding: '16px 0',
        marginBottom: 16,
        borderBottom: '1px solid var(--border)',
      }}>
        <h3 style={{ fontSize: 14, fontWeight: 700, marginBottom: 8 }}>Push Notifications</h3>
        <span style={{ fontSize: 13, color: 'var(--muted)' }}>
          Push notifications are not supported in this browser.
        </span>
      </div>
    )
  }

  const isGranted = permission === 'granted'
  const isDenied = permission === 'denied'
  const isLoading = subscribeMut.isPending || unsubscribeMut.isPending || loadingKey

  const handleEnable = () => {
    if (!vapidData?.publicKey) return
    subscribeMut.mutate(vapidData.publicKey)
  }

  const handleDisable = () => {
    unsubscribeMut.mutate()
  }

  const permissionLabel = isGranted
    ? 'Enabled'
    : isDenied
      ? 'Blocked'
      : 'Not enabled'

  const permissionColor = isGranted
    ? 'var(--ok)'
    : isDenied
      ? 'var(--crit)'
      : 'var(--muted)'

  const error = subscribeMut.error || unsubscribeMut.error

  return (
    <div style={{
      padding: '16px 0',
      marginBottom: 16,
      borderBottom: '1px solid var(--border)',
    }}>
      <div style={{ display: 'flex', alignItems: 'center', gap: 12, marginBottom: 12 }}>
        <h3 style={{ fontSize: 14, fontWeight: 700 }}>Push Notifications</h3>
        <span style={{
          display: 'inline-block',
          padding: '2px 10px',
          borderRadius: 4,
          fontSize: 11,
          fontWeight: 600,
          background: isGranted ? 'rgba(34,197,94,.1)' : isDenied ? 'rgba(239,68,68,.1)' : 'rgba(255,255,255,.05)',
          color: permissionColor,
        }}>
          {permissionLabel}
        </span>
      </div>

      {error && (
        <p style={{ fontSize: 12, color: 'var(--crit)', marginBottom: 8 }}>
          {error.message}
        </p>
      )}

      {subscribeMut.isSuccess && (
        <p style={{ fontSize: 12, color: 'var(--ok)', marginBottom: 8 }}>
          Push notifications enabled successfully.
        </p>
      )}

      {unsubscribeMut.isSuccess && (
        <p style={{ fontSize: 12, color: 'var(--ok)', marginBottom: 8 }}>
          Push notifications disabled successfully.
        </p>
      )}

      <div style={{ display: 'flex', gap: 8, alignItems: 'center' }}>
        {!isGranted && !isDenied && (
          <button
            style={{ ...btnStyle, borderColor: 'var(--ok)', color: 'var(--ok)' }}
            onClick={handleEnable}
            disabled={isLoading}
          >
            {isLoading ? 'Enabling...' : 'Enable Push'}
          </button>
        )}
        {isGranted && (
          <button
            style={{ ...btnStyle, borderColor: 'var(--crit)', color: 'var(--crit)' }}
            onClick={handleDisable}
            disabled={isLoading}
          >
            {isLoading ? 'Disabling...' : 'Disable Push'}
          </button>
        )}
        {isDenied && (
          <span style={{ fontSize: 12, color: 'var(--muted)' }}>
            Push notifications are blocked. Please enable them in your browser settings.
          </span>
        )}
      </div>
    </div>
  )
}
