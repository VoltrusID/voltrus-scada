const SW_PATH = '/sw.js';

export async function registerServiceWorker(): Promise<boolean> {
  if (!('serviceWorker' in navigator)) {
    console.log('[SW] Service workers not supported');
    return false;
  }

  try {
    await navigator.serviceWorker.register(SW_PATH, { scope: '/' });
    console.log('[SW] Service worker registered');
    return true;
  } catch (err) {
    console.error('[SW] Service worker registration failed:', err);
    return false;
  }
}
