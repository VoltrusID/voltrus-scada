// Service worker for Voltrus (docs/designer.md, phase 5): keeps the app
// shell so the installed app opens without a network, where it shows the
// last-known values of the last screen with their age. Alarm notifications
// on phones come from the Voltrus SCADA app (docs/mobile.md), not from Web
// Push.
//
// * Pages: network first; the cached shell when the server cannot be reached.
// * Built assets (/assets/, named by content hash): cache first.
// * Translations, icons, the static manifest: network first, cache fallback.
// * The API (/api/) and everything else: never cached, never answered here.

const SHELL = 'voltrus-shell-v1';
const ASSETS = 'voltrus-assets-v1';
const KEEP = [SHELL, ASSETS];
const PRECACHE = ['/', '/manifest.json', '/favicon-32.png', '/favicon-192.png', '/favicon-512.png'];
/** Most built files kept; older ones are dropped first. */
const MAX_ASSETS = 300;

self.addEventListener('install', (event) => {
  event.waitUntil(
    caches.open(SHELL)
      .then((c) => c.addAll(PRECACHE))
      .catch(() => {})
      .then(() => self.skipWaiting()),
  );
});

self.addEventListener('activate', (event) => {
  event.waitUntil(
    caches.keys()
      .then((keys) => Promise.all(keys.filter((k) => !KEEP.includes(k)).map((k) => caches.delete(k))))
      .then(() => self.clients.claim()),
  );
});

async function trim(cacheName, max) {
  const c = await caches.open(cacheName);
  const keys = await c.keys();
  for (let i = 0; i < keys.length - max; i++) await c.delete(keys[i]);
}

async function networkFirst(request, cacheName, fallbackUrl) {
  try {
    const res = await fetch(request);
    if (res.ok && res.type === 'basic') {
      const copy = res.clone();
      caches.open(cacheName).then((c) => c.put(fallbackUrl || request, copy)).catch(() => {});
    }
    return res;
  } catch (err) {
    const hit = await caches.match(fallbackUrl || request);
    if (hit) return hit;
    throw err;
  }
}

async function cacheFirst(request) {
  const hit = await caches.match(request);
  if (hit) return hit;
  const res = await fetch(request);
  if (res.ok && res.type === 'basic') {
    const copy = res.clone();
    caches.open(ASSETS).then((c) => c.put(request, copy)).then(() => trim(ASSETS, MAX_ASSETS)).catch(() => {});
  }
  return res;
}

self.addEventListener('fetch', (event) => {
  const req = event.request;
  if (req.method !== 'GET') return;
  const url = new URL(req.url);
  if (url.origin !== self.location.origin) return;
  if (url.pathname.startsWith('/api/') || url.pathname === '/metrics') return;
  if (req.mode === 'navigate') {
    // Every page of the single-page app is the same shell.
    event.respondWith(networkFirst(req, SHELL, '/'));
    return;
  }
  if (url.pathname.startsWith('/assets/')) {
    event.respondWith(cacheFirst(req));
    return;
  }
  if (url.pathname.startsWith('/locales/') || PRECACHE.includes(url.pathname) || url.pathname === '/favicon.ico' || url.pathname === '/favicon-16.png') {
    event.respondWith(networkFirst(req, SHELL));
  }
});
