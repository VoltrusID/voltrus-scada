// Service worker for Voltrus — Web Push Notifications

self.addEventListener("install", () => {
  console.log("[SW] Installed");
  self.skipWaiting();
});

self.addEventListener("activate", (event) => {
  console.log("[SW] Activated");
  event.waitUntil(self.clients.claim());
});

self.addEventListener("push", (event) => {
  let data = { title: "Voltrus", body: "New notification" };

  if (event.data) {
    try {
      data = event.data.json();
    } catch {
      data.body = event.data.text();
    }
  }

  const options = {
    body: data.body,
    icon: "/favicon-32x32.png",
    badge: "/favicon-16x16.png",
    tag: data.source_key || "default",
    data: {
      url: self.location.origin,
      source_key: data.source_key,
      severity: data.severity,
    },
    // Vibrate pattern: [wait, vibrate, wait, vibrate]
    vibrate:
      data.severity === "critical" ? [200, 100, 200, 100, 200] : [100, 50],
  };

  event.waitUntil(self.registration.showNotification(data.title, options));
});

self.addEventListener("notificationclick", (event) => {
  event.notification.close();

  const rawUrl = event.notification.data?.url || self.location.origin;
  // Validate URL origin to prevent open-ended redirects
  const url = rawUrl.startsWith(self.location.origin)
    ? rawUrl
    : self.location.origin;

  event.waitUntil(
    self.clients
      .matchAll({ type: "window", includeUncontrolled: true })
      .then((clients) => {
        // Focus existing window if open
        for (const client of clients) {
          if (client.url === url && "focus" in client) {
            return client.focus();
          }
        }
        // Otherwise open new window
        return self.clients.openWindow(url);
      }),
  );
});
