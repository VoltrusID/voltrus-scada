const SW_PATH = '/sw.js';

let registration: ServiceWorkerRegistration | null = null;

export async function registerServiceWorker(): Promise<boolean> {
  if (!('serviceWorker' in navigator)) {
    console.log('[Push] Service workers not supported');
    return false;
  }

  try {
    registration = await navigator.serviceWorker.register(SW_PATH, { scope: '/' });
    console.log('[Push] Service worker registered');
    return true;
  } catch (err) {
    console.error('[Push] Service worker registration failed:', err);
    return false;
  }
}

function base64UrlToUint8Array(base64Url: string): BufferSource {
  const padding = '='.repeat((4 - (base64Url.length % 4)) % 4);
  const base64 = (base64Url + padding).replace(/-/g, '+').replace(/_/g, '/');
  const binary = atob(base64);
  const bytes = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i++) {
    bytes[i] = binary.charCodeAt(i);
  }
  return bytes;
}

export async function subscribePush(vapidPublicKey: string): Promise<boolean> {
  if (!registration) {
    console.error('[Push] No service worker registration');
    return false;
  }

  try {
    const permission = await Notification.requestPermission();
    if (permission !== 'granted') {
      console.log('[Push] Notification permission denied');
      return false;
    }

    const subscription = await registration.pushManager.subscribe({
      userVisibleOnly: true,
      applicationServerKey: base64UrlToUint8Array(vapidPublicKey),
    });

    const json = subscription.toJSON();
    const resp = await fetch('/api/v1/push/subscribe', {
      method: 'POST',
      credentials: 'include',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({
        endpoint: subscription.endpoint,
        p256dh: json.keys?.p256dh || '',
        auth_key: json.keys?.auth || '',
      }),
    });

    if (!resp.ok) {
      console.error('[Push] Subscribe API failed:', resp.status);
      return false;
    }

    console.log('[Push] Subscribed successfully');
    return true;
  } catch (err) {
    console.error('[Push] Subscription failed:', err);
    return false;
  }
}

export async function unsubscribePush(): Promise<boolean> {
  if (!registration) return false;

  try {
    const subscription = await registration.pushManager.getSubscription();
    if (!subscription) return true;

    const resp = await fetch('/api/v1/push/subscribe', {
      method: 'DELETE',
      credentials: 'include',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ endpoint: subscription.endpoint }),
    });

    await subscription.unsubscribe();

    if (!resp.ok) {
      console.error('[Push] Unsubscribe API failed:', resp.status);
      return false;
    }

    console.log('[Push] Unsubscribed successfully');
    return true;
  } catch (err) {
    console.error('[Push] Unsubscription failed:', err);
    return false;
  }
}

export function getPushPermissionState(): NotificationPermission | 'unsupported' {
  if (!('Notification' in window)) return 'unsupported';
  return Notification.permission;
}

export function isPushSupported(): boolean {
  return 'serviceWorker' in navigator && 'PushManager' in window;
}
