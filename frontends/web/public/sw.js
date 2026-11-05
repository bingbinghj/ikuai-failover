/* Browser notifications only: this worker does not intercept or cache app requests. */
self.addEventListener('install', event => { event.waitUntil(self.skipWaiting()) })
self.addEventListener('activate', event => { event.waitUntil(self.clients.claim()) })

function appUrl(value) {
  try {
    const url = new URL(typeof value === 'string' ? value : '/', self.location.origin)
    if (url.origin !== self.location.origin || !['http:', 'https:'].includes(url.protocol)) return self.location.origin + '/'
    // /notifications is the JSON API; notification clicks should open the application.
    if (url.pathname === '/notifications' || url.pathname.startsWith('/notifications/')) return self.location.origin + '/'
    return url.href
  } catch { return self.location.origin + '/' }
}

self.addEventListener('push', event => {
  let payload = {}
  if (event.data) {
    try { payload = event.data.json() } catch { payload = { body: event.data.text() } }
  }
  if (!payload || typeof payload !== 'object' || Array.isArray(payload)) payload = {}
  const title = typeof payload.title === 'string' && payload.title.trim() ? payload.title : 'ikuai-failover'
  const body = typeof payload.body === 'string' ? payload.body : typeof payload.text === 'string' ? payload.text : '你有一条新的通知。'
  const options = { body, data: { url: appUrl(payload.url) } }
  if (typeof payload.id === 'string' && payload.id) options.tag = payload.id
  event.waitUntil(self.registration.showNotification(title, options))
})

self.addEventListener('notificationclick', event => {
  event.notification.close()
  const url = appUrl(event.notification.data && event.notification.data.url)
  event.waitUntil((async () => {
    const windows = await self.clients.matchAll({ type: 'window', includeUncontrolled: true })
    const current = windows.find(client => {
      try { return new URL(client.url).origin === self.location.origin } catch { return false }
    })
    if (current) {
      if (current.url !== url && typeof current.navigate === 'function') {
        const navigated = await current.navigate(url)
        return (navigated || current).focus()
      }
      return current.focus()
    }
    return self.clients.openWindow(url)
  })())
})
