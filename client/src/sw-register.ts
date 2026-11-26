// The service worker (public/sw.js) keeps the app shell for offline use and
// the web app manifest makes the HMI installable on a phone's home screen
// (docs/designer.md, phase 5). Alarm notifications on phones come from the
// Voltrus SCADA app (docs/mobile.md), not from the browser.

const SW_PATH = '/sw.js';

/** The manifest before sign-in: Voltrus' own, no site name. */
export const STATIC_MANIFEST = '/manifest.json';
/** The site's manifest (name, white-label logo and accent); needs a sign-in. */
export const SITE_MANIFEST = '/api/v1/site/manifest.webmanifest';

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

/**
 * Point the page's manifest link at the site's manifest once someone is
 * signed in (the request carries the session cookie), and back at the static
 * one after sign-out. The browser reads the manifest again when the link
 * changes, so an install after sign-in carries the site's name and branding.
 */
export function applySiteManifest(doc: Document, signedIn: boolean): void {
  let link = doc.querySelector<HTMLLinkElement>('link[rel="manifest"]');
  if (!link) {
    link = doc.createElement('link');
    link.rel = 'manifest';
    doc.head.appendChild(link);
  }
  const href = signedIn ? SITE_MANIFEST : STATIC_MANIFEST;
  if (signedIn) link.setAttribute('crossorigin', 'use-credentials');
  else link.removeAttribute('crossorigin');
  if (link.getAttribute('href') !== href) link.setAttribute('href', href);
}
