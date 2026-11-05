import { computed, ref, toValue, type MaybeRefOrGetter } from 'vue'

type Request = (path: string, init?: RequestInit) => Promise<unknown>
export type BrowserPushStatus = 'idle' | 'working' | 'enabled' | 'denied' | 'unsupported' | 'error'

function object(value: unknown): Record<string, unknown> {
  return value !== null && typeof value === 'object' && !Array.isArray(value) ? value as Record<string, unknown> : {}
}
function keyBytes(value: unknown): ArrayBuffer {
  if (typeof value !== 'string' || !/^[A-Za-z0-9_-]+$/.test(value)) throw new Error('无法读取浏览器通知密钥，请重试。')
  const encoded = value.replace(/-/g, '+').replace(/_/g, '/')
  const binary = atob(encoded + '='.repeat((4 - encoded.length % 4) % 4))
  const bytes = new Uint8Array(binary.length)
  for (let index = 0; index < binary.length; index += 1) bytes[index] = binary.charCodeAt(index)
  if (bytes.length !== 65 || bytes[0] !== 4) throw new Error('浏览器通知密钥无效，请重新添加此渠道。')
  return bytes.buffer
}
function sameKey(left: ArrayBuffer | null, right: ArrayBuffer) {
  if (!left || left.byteLength !== right.byteLength) return false
  const first = new Uint8Array(left), second = new Uint8Array(right)
  return first.every((value, index) => value === second[index])
}
async function activate(registration: ServiceWorkerRegistration) {
  if (registration.active?.state === 'activated') return
  const worker = registration.installing ?? registration.waiting ?? registration.active
  if (!worker) throw new Error('浏览器通知服务未启动，请刷新页面后重试。')
  await new Promise<void>((resolve, reject) => {
    const cleanup = () => { clearTimeout(timeout); worker.removeEventListener('statechange', changed) }
    const changed = () => {
      if (worker.state === 'activated') { cleanup(); resolve() }
      else if (worker.state === 'redundant') { cleanup(); reject(new Error('浏览器通知服务未启动，请重试。')) }
    }
    const timeout = setTimeout(() => { cleanup(); reject(new Error('浏览器通知服务启动超时，请重试。')) }, 20000)
    worker.addEventListener('statechange', changed)
    changed()
  })
}
function errorMessage(reason: unknown) {
  const data = object(reason), payload = object(data.payload)
  if (data.status === 401) return '登录已过期，请重新登录。'
  if (typeof payload.message === 'string') return payload.message
  if (reason instanceof DOMException && reason.name === 'NotAllowedError') return '浏览器未允许通知，请在网站权限设置中允许通知后再试。'
  if (reason instanceof DOMException && reason.name === 'NotSupportedError') return '此浏览器暂不支持浏览器推送。'
  if (reason instanceof DOMException && reason.name === 'SecurityError') return '请通过 HTTPS 或 localhost 开启浏览器通知。'
  if (reason instanceof DOMException) return '浏览器通知操作未完成，请刷新页面后重试。'
  if (reason instanceof Error && /[\u4e00-\u9fff]/.test(reason.message)) return reason.message
  return '开启浏览器通知失败，请检查浏览器权限和网络连接。'
}

/** Enrollment runs only from enable(), which must be called by the user's button click. */
export function useBrowserPush(request: Request, tag: MaybeRefOrGetter<string>, username: MaybeRefOrGetter<string>) {
  const supported = computed(() => typeof window !== 'undefined' && window.isSecureContext && typeof Notification !== 'undefined' && 'serviceWorker' in navigator && 'PushManager' in window)
  const supportMessage = computed(() => typeof window === 'undefined' || !window.isSecureContext ? '浏览器通知需要通过 HTTPS 或 localhost 访问。' : supported.value ? '' : '此浏览器暂不支持浏览器推送。')
  const busy = ref(false), enabled = ref(false), message = ref('')
  const status = ref<BrowserPushStatus>('idle')
  const restartRequired = ref(false), savedRevision = ref('')
  const statusLabel = computed(() => ({ idle: '未开启', working: '正在开启', enabled: '已开启', denied: '权限未允许', unsupported: '暂不支持', error: '开启失败' }[status.value]))

  async function enable(): Promise<boolean> {
    if (busy.value) return false
    message.value = ''; restartRequired.value = false; enabled.value = false
    if (!supported.value) { status.value = 'unsupported'; message.value = supportMessage.value; return false }
    const channelTag = String(toValue(tag) ?? '').trim(), account = String(toValue(username) ?? '').trim()
    if (!/^[A-Za-z0-9_-]{1,64}$/.test(channelTag) || !account || new TextEncoder().encode(account).length > 256 || Array.from(account).some(character => { const code = character.charCodeAt(0); return code < 32 || code >= 127 && code <= 159 })) { status.value = 'error'; message.value = '请先填写接收浏览器通知的用户名，并保存此渠道。'; return false }
    if (Notification.permission === 'denied') { status.value = 'denied'; message.value = '请在浏览器的网站权限设置中允许通知。'; return false }
    busy.value = true; status.value = 'working'
    try {
      // Request permission before the first await, while the button's user activation is live.
      const permission = Notification.permission === 'granted' ? Promise.resolve('granted' as NotificationPermission) : Notification.requestPermission()
      if (await permission !== 'granted') { status.value = 'denied'; message.value = '浏览器通知尚未获得允许，可以再次点击开启。'; return false }
      const [keyResult, registration] = await Promise.all([
        request('/notifications/webpush/key', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ tag: channelTag }) }),
        navigator.serviceWorker.register('/sw.js', { scope: '/', updateViaCache: 'none' }),
      ])
      const key = object(keyResult)
      if (key.success !== true) throw new Error(typeof key.message === 'string' ? key.message : '无法初始化浏览器通知，请先保存此渠道。')
      const applicationServerKey = keyBytes(key.public_key)
      await activate(registration)
      let subscription = await registration.pushManager.getSubscription()
      if (subscription && (!sameKey(subscription.options.applicationServerKey, applicationServerKey) || subscription.expirationTime !== null && subscription.expirationTime <= Date.now())) {
        if (!await subscription.unsubscribe()) throw new Error('无法更新此浏览器的订阅，请刷新后重试。')
        subscription = null
      }
      if (!subscription) subscription = await registration.pushManager.subscribe({ userVisibleOnly: true, applicationServerKey })
      const result = object(await request('/notifications/webpush/subscribe', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ tag: channelTag, username: account, subscription: subscription.toJSON() }) }))
      if (result.success !== true) throw new Error(typeof result.message === 'string' ? result.message : '浏览器订阅未能保存，请重试。')
      enabled.value = true; status.value = 'enabled'
      restartRequired.value = result.restart_required === true
      savedRevision.value = typeof result.revision === 'string' ? result.revision : ''
      message.value = typeof result.message === 'string' ? result.message : restartRequired.value ? '浏览器通知已开启，重启服务后使用新订阅。' : '浏览器通知已开启。'
      return true
    } catch (reason: unknown) { enabled.value = false; status.value = 'error'; message.value = errorMessage(reason); return false }
    finally { busy.value = false }
  }
  return { supported, supportMessage, busy, enabled, status, statusLabel, message, restartRequired, savedRevision, enable }
}
