import { computed, onMounted, onUnmounted, ref } from 'vue'
import { object, parseNotices, parseSnapshot, type IkuaiSettings, type IkuaiSettingsUpdate, type Notice, type Session, type Snapshot } from './model'

class HttpError extends Error {
  constructor(public status: number, public payload: Record<string, unknown> = {}) { super('请求失败') }
}

export function useDashboard() {
  const session = ref<Session | null>(null)
  const booting = ref(true)
  const authenticating = ref(false)
  const authError = ref('')
  const status = ref<Snapshot | null>(null)
  const notices = ref<Notice[]>([])
  const loading = ref(false)
  const paused = ref(false)
  const statusError = ref('')
  const historyError = ref('')
  const updatedAt = ref(0)
  const ikuaiSettings = ref<IkuaiSettings | null>(null)
  const settingsLoading = ref(false)
  const settingsSaving = ref(false)
  const settingsError = ref('')
  const settingsSaved = ref(false)
  const restarting = ref(false)
  const restartError = ref('')
  const authenticated = computed(() => session.value !== null)
  let timer: ReturnType<typeof setInterval> | undefined
  let generation = 0
  let refreshController: AbortController | null = null
  let authorization = ''

  async function request(path: string, init: RequestInit = {}, signal?: AbortSignal): Promise<unknown> {
    const headers = new Headers(init.headers)
    headers.set('Accept', 'application/json')
    if (authorization) headers.set('Authorization', authorization)
    const timeout = path.startsWith('/notifications/clawbot/') ? 90000 : path === '/wan/selection' ? 125000 : path === '/notifications/test' ? 90000 : path === '/settings/options' ? 45000 : 8000
    const response = await fetch(path, { ...init, credentials: 'same-origin', headers, signal: signal ? AbortSignal.any([signal, AbortSignal.timeout(timeout)]) : AbortSignal.timeout(timeout) })
    if (!response.ok) {
      let payload: Record<string, unknown> = {}
      try { payload = object(await response.json()) } catch { /* Empty error responses need only their status. */ }
      throw new HttpError(response.status, payload)
    }
    if (response.status === 204) return {}
    return response.json() as Promise<unknown>
  }

  function clearSession() {
    generation += 1
    authorization = ''
    refreshController?.abort()
    session.value = null
    status.value = null
    notices.value = []
    statusError.value = ''
    historyError.value = ''
    updatedAt.value = 0
    loading.value = false
    ikuaiSettings.value = null
    settingsError.value = ''
    settingsLoading.value = false
    settingsSaving.value = false
    settingsSaved.value = false
  }

  function settingsFailure(error: unknown) {
    if (error instanceof HttpError && error.status === 401) {
      clearSession()
      authError.value = '登录已过期，请重新登录。'
      return
    }
    settingsError.value = error instanceof HttpError
      ? error.status === 409 ? '配置已被其他方式修改，请重新加载后保存。'
        : error.status === 400 ? '请检查 HTTPS 地址、账号长度和超时时间。'
          : '无法读写配置，请确认配置目录可写后重试。'
      : '连接失败或超时，请稍后重试。'
  }

  async function loadIkuaiSettings() {
    if (!authenticated.value || settingsLoading.value || settingsSaving.value) return
    const version = generation
    settingsLoading.value = true
    settingsError.value = ''
    try {
      const value = object(await request('/settings/ikuai'))
      if (version !== generation) return
      if (typeof value.revision !== 'string' || typeof value.base_url !== 'string') throw new Error('设置数据不可用')
      const overrides = object(value.environment_overrides)
      ikuaiSettings.value = {
        revision: value.revision, base_url: value.base_url,
        username: typeof value.username === 'string' ? value.username : '',
        password_configured: value.password_configured === true,
        allow_invalid_certificate: value.allow_invalid_certificate === true,
        timeout_ms: typeof value.timeout_ms === 'number' ? value.timeout_ms : 5000,
        environment_overrides: { username: overrides.username === true, password: overrides.password === true },
      }
    } catch (error: unknown) { if (version === generation) settingsFailure(error) }
    finally { if (version === generation) settingsLoading.value = false }
  }

  async function saveIkuaiSettings(value: IkuaiSettingsUpdate) {
    if (!authenticated.value || settingsSaving.value || settingsLoading.value) return false
    const version = generation
    settingsSaving.value = true
    settingsError.value = ''
    try {
      const result = object(await request('/settings/ikuai', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(value) }))
      if (version !== generation) return false
      if (result.success !== true) throw new Error('保存失败')
      settingsSaved.value = true
      settingsSaving.value = false
      await loadIkuaiSettings()
      return true
    } catch (error: unknown) { if (version === generation) settingsFailure(error); return false }
    finally { if (version === generation) settingsSaving.value = false }
  }

  async function refresh() {
    if (!authenticated.value || loading.value || restarting.value) return
    const revision = generation
    loading.value = true
    refreshController = new AbortController()
    const result = await Promise.allSettled([request('/status', {}, refreshController.signal), request('/notifications', {}, refreshController.signal)])
    if (revision !== generation) return
    if (result.some(item => item.status === 'rejected' && item.reason instanceof HttpError && item.reason.status === 401)) {
      clearSession()
      authError.value = '登录已过期，请重新登录。'
      return
    }
    const [snapshot, history] = result
    if (snapshot?.status === 'fulfilled') {
      try { status.value = parseSnapshot(snapshot.value); updatedAt.value = Date.now(); statusError.value = '' } catch { statusError.value = '状态数据暂不可用，请稍后刷新。' }
    } else { statusError.value = '暂时无法获取网络状态，正在保留上次结果。' }
    if (history?.status === 'fulfilled') {
      notices.value = parseNotices(history.value)
      historyError.value = ''
    } else { historyError.value = '通知记录暂不可用，请稍后刷新。' }
    loading.value = false
  }

  async function bootstrap() {
    booting.value = true
    try {
      const result = object(await request('/auth/session'))
      if (result.authenticated === true) {
        session.value = { username: typeof result.username === 'string' ? result.username : '管理员' }
        await refresh()
      } else { clearSession() }
    } catch (error: unknown) {
      if (error instanceof HttpError && error.status === 401) clearSession()
      else authError.value = '暂时无法连接服务，请稍后登录。'
    } finally { booting.value = false }
  }

  async function login(username: string, password: string, remember = true) {
    if (authenticating.value) return false
    authenticating.value = true
    authError.value = ''
    try {
      authorization = 'Basic ' + btoa(Array.from(new TextEncoder().encode(`${username}:${password}`), byte => String.fromCharCode(byte)).join(''))
      const result = object(await request('/auth/login', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ remember }) }))
      if (result.authenticated !== true) throw new HttpError(401)
      session.value = { username: typeof result.username === 'string' ? result.username : username }
      authorization = ''
      await refresh()
      return true
    } catch (error: unknown) {
      authorization = ''
      authError.value = error instanceof HttpError ? error.status === 401 ? '账号或密码不正确，请重试。' : error.status === 403 ? '该账号无法登录此服务。' : error.status >= 500 ? '服务暂时不可用，请稍后重试。' : '无法完成登录，请检查账号和密码。' : '连接失败或超时，请稍后重试。'
      return false
    } finally { authenticating.value = false }
  }

  async function logout() {
    const pending = request('/auth/logout', { method: 'POST' })
    clearSession()
    authError.value = ''
    try { await pending } catch { authError.value = '已退出当前页面；服务暂时无法响应。' }
  }

  async function authenticatedRequest(path: string, init: RequestInit = {}) {
    try { return await request(path, init) }
    catch (error: unknown) {
      if (error instanceof HttpError && error.status === 401) {
        clearSession()
        authError.value = '登录已过期，请重新登录。'
      }
      throw error
    }
  }

  async function restartService() {
    if (restarting.value) return
    restartError.value = ''
    try { await authenticatedRequest('/system/restart', { method: 'POST' }) }
    catch { restartError.value = '未能请求重启，请稍后重试。'; return }
    restarting.value = true
    refreshController?.abort()
    loading.value = false
    await new Promise(resolve => setTimeout(resolve, 2500))
    for (let attempt = 0; attempt < 30; attempt += 1) {
      try {
        await request('/healthz')
        restarting.value = false
        settingsSaved.value = false
        await bootstrap()
        if (!session.value) { clearSession(); authError.value = '服务已重启，请使用当前配置中的网页登录账号。' }
        return
      } catch { await new Promise(resolve => setTimeout(resolve, 2000)) }
    }
    restarting.value = false
    restartError.value = '服务尚未恢复，请检查容器的自动重启设置后刷新页面。'
  }

  onMounted(() => {
    void bootstrap()
    timer = setInterval(() => { if (!paused.value && !document.hidden) void refresh() }, 5000)
  })
  onUnmounted(() => { if (timer) clearInterval(timer); refreshController?.abort(); generation += 1 })
  return { session, booting, authenticating, authError, authenticated, status, notices, loading, paused, statusError, historyError, updatedAt, refresh, login, logout, ikuaiSettings, settingsLoading, settingsSaving, settingsError, settingsSaved, loadIkuaiSettings, saveIkuaiSettings, authenticatedRequest, restartService, restarting, restartError }
}
