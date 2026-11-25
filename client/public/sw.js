// Service worker for Voltrus. Alarm notifications on phones come from the
// Voltrus SCADA app (docs/mobile.md), not from Web Push.

self.addEventListener("install", () => {
  console.log("[SW] Installed");
  self.skipWaiting();
});

self.addEventListener("activate", (event) => {
  console.log("[SW] Activated");
  event.waitUntil(self.clients.claim());
});
