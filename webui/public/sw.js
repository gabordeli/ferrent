const CACHE = "ferrent-web-v1";
const API_CACHE = "ferrent-web-api-v1";
const SHELL = [
  "/",
  "/index.html",
  "/manifest.webmanifest",
  "/favicon.svg",
  "/icons/icon-192.png",
  "/icons/icon-512.png",
  "/icons/apple-touch-icon.png",
];

// Only these API endpoints are safe to serve from the cache when the daemon is
// unreachable; the rest must fail loudly.
const CACHEABLE_API = ["/api/torrents", "/api/session", "/api/labels"];

self.addEventListener("install", (event) => {
  event.waitUntil(
    caches
      .open(CACHE)
      .then((cache) => cache.addAll(SHELL))
      .then(() => self.skipWaiting()),
  );
});

self.addEventListener("activate", (event) => {
  event.waitUntil(
    caches
      .keys()
      .then((keys) =>
        Promise.all(
          keys
            .filter((key) => key !== CACHE && key !== API_CACHE)
            .map((key) => caches.delete(key)),
        ),
      )
      .then(() => self.clients.claim()),
  );
});

function isCacheableApi(pathname) {
  return CACHEABLE_API.some(
    (endpoint) =>
      pathname === endpoint || pathname.startsWith(`${endpoint}/`),
  );
}

function isShellAsset(pathname) {
  return (
    pathname.startsWith("/assets/") ||
    pathname.startsWith("/icons/") ||
    pathname === "/favicon.svg" ||
    pathname === "/manifest.webmanifest"
  );
}

async function cachePut(cacheName, request, response) {
  const cache = await caches.open(cacheName);
  await cache.put(request, response);
}

async function cacheFirst(request) {
  const cached = await caches.match(request);
  if (cached) return cached;
  const response = await fetch(request);
  if (response.ok) await cachePut(CACHE, request, response.clone());
  return response;
}

async function networkFirstShell(request) {
  try {
    const response = await fetch(request);
    if (response.ok) await cachePut(CACHE, "/index.html", response.clone());
    return response;
  } catch {
    const cached = await caches.match("/index.html");
    return cached || Response.error();
  }
}

async function cachedShell() {
  const cached = await caches.match("/index.html");
  if (cached) return cached;
  return fetch("/index.html");
}

async function apiResponse(request) {
  try {
    const response = await fetch(request);
    if (response.ok) await cachePut(API_CACHE, request, response.clone());
    return response;
  } catch {
    const cached = await caches.match(request);
    if (!cached) return Response.error();
    const headers = new Headers(cached.headers);
    headers.set("X-MobileUi-Stale", "1");
    return new Response(await cached.blob(), {
      status: cached.status,
      statusText: cached.statusText,
      headers,
    });
  }
}

self.addEventListener("fetch", (event) => {
  const { request } = event;
  if (request.method !== "GET") return;
  const url = new URL(request.url);
  if (url.origin !== self.location.origin) return;

  if (url.pathname.startsWith("/api/")) {
    if (isCacheableApi(url.pathname)) {
      event.respondWith(apiResponse(request));
    }
    return;
  }

  if (request.mode === "navigate") {
    event.respondWith(
      url.pathname === "/share"
        ? cachedShell()
        : networkFirstShell(request),
    );
    return;
  }

  if (isShellAsset(url.pathname)) {
    event.respondWith(cacheFirst(request));
  }
});

self.addEventListener("message", (event) => {
  // Only the app itself may clear the API cache.
  if (event.origin !== self.location.origin) return;
  if (event.data?.type === "clear-api-cache") {
    event.waitUntil(caches.delete(API_CACHE));
  }
});

function notificationContent(payload, hu) {
  const fallback = hu ? "Hiba" : "Error";
  if (payload.kind === "finished") {
    return {
      title: hu ? "Torrent kész" : "Torrent finished",
      body: payload.name || payload.body || "",
    };
  }
  if (payload.kind === "error") {
    const detail = payload.name
      ? `${payload.name}: ${payload.message || fallback}`
      : payload.body || "";
    return {
      title: hu ? "Torrent hiba" : "Torrent error",
      body: detail,
    };
  }
  if (payload.kind === "test") {
    return {
      title: hu ? "Deluge Mobile teszt" : "Deluge Mobile test",
      body: hu
        ? "A push értesítések működnek."
        : "Push notifications are working.",
    };
  }
  return {
    title: payload.title || "Deluge Mobile",
    body: payload.body || "",
  };
}

self.addEventListener("push", (event) => {
  let payload = {};
  try {
    payload = event.data ? event.data.json() : {};
  } catch {
    payload = { body: event.data ? event.data.text() : "" };
  }
  const language = (self.navigator?.language || "en").toLowerCase();
  const hu = language.startsWith("hu");
  const { title, body } = notificationContent(payload, hu);
  const url = payload.url || "/";
  event.waitUntil(
    self.registration.showNotification(title, {
      body,
      tag: payload.tag,
      icon: "/icons/icon-192.png",
      badge: "/icons/icon-192.png",
      data: { url },
    }),
  );
});

self.addEventListener("notificationclick", (event) => {
  event.notification.close();
  const target = new URL(
    event.notification.data?.url || "/",
    self.location.origin,
  ).href;
  event.waitUntil(
    self.clients
      .matchAll({ type: "window", includeUncontrolled: true })
      .then((clients) => {
        for (const client of clients) {
          if (client.url.startsWith(self.location.origin) && "focus" in client) {
            if ("navigate" in client) client.navigate(target);
            return client.focus();
          }
        }
        return self.clients.openWindow(target);
      }),
  );
});
