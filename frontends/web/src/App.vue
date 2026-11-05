<script setup lang="ts">
import { computed, nextTick, onMounted, onUnmounted, ref, watch } from 'vue'
import { useDisplay } from 'vuetify'
import Icon from './Icon.vue'
import ConfigEditor from './ConfigEditor.vue'
import CardSettingsPanel from './CardSettingsPanel.vue'
import LogsCenter from './LogsCenter.vue'
import { useDashboard } from './useDashboard'
import { useNavigationState } from './navigationState'
import { useRememberedLogin } from './useRememberedLogin'
import { useAppearance } from './theme'
import { useNotificationReadState } from './notificationState'
import { notificationProviderIcon } from './notificationProviderIcons'
import { healthLabel, object, providerLabel, timeLabel } from './model'

const { session, booting, authenticating, authError, authenticated, status, notices, loading, paused, statusError, historyError, updatedAt, refresh, login, logout, authenticatedRequest, restartService, restarting, restartError } = useDashboard()
type Section = 'overview' | 'settings' | 'logs'
type LogView = 'realtime' | 'history' | 'notifications'
const { section, settingsGroup, logView, navigate: changePage, selectSettings } = useNavigationState()
const drawerView = ref<LogView>('realtime')
const drawerTitle = computed(() => ({ realtime: '实时日志', history: '历史日志', notifications: '通知记录' }[drawerView.value]))
const navigation: { key: Section; title: string; icon: string; description: string }[] = [
  { key: 'overview', title: '总览', icon: 'dashboard', description: '查看线路状态、当前出口和故障转移结果。' },
  { key: 'settings', title: '配置', icon: 'settings', description: '从爱快选择接口和规则，按需配置通知。' },
]
const page = computed(() => section.value === 'logs' ? { title: '实时日志' } : navigation.find(item => item.key === section.value) ?? navigation[0]!)
const mobileNavigation = ref(false)
const railNavigation = ref(false)
const { mdAndUp } = useDisplay()
const drawerVisible = computed({ get: () => mdAndUp.value || mobileNavigation.value, set: value => { mobileNavigation.value = value } })
const themeMenu = computed({ get: () => activePopover.value === 'theme', set: value => { activePopover.value = value ? 'theme' : '' } })
const profileMenu = computed({ get: () => activePopover.value === 'profile', set: value => { activePopover.value = value ? 'profile' : '' } })
const activePopover = ref<'theme' | 'profile' | ''>('')
const notificationOpen = ref(false)
const restartConfirmationOpen = ref(false)
const modalKind = ref<'config' | 'health' | 'selection'>('config')
type CardPanelKind = 'connection' | 'lines' | 'rules' | 'ipv6' | 'notifications' | 'timing'
type CardPanelControls = { save: () => unknown; canSave: boolean; dirty: boolean; saving: boolean }
const panelKind = ref<CardPanelKind>('connection')
const cardPanel = ref<CardPanelControls | null>(null)
const selectedLine = ref('')
const dialogOpen = ref(false)
const selectionMode = ref<'auto' | 'primary' | 'backup'>('auto')
const selectionLoading = ref(false)
const selectionSaving = ref(false)
const selectionLoaded = ref(false)
const selectionError = ref('')
const modalTitle = computed(() => modalKind.value === 'health' ? '线路健康' : modalKind.value === 'selection' ? '选择当前出口' : selectedLine.value ? lineLabel(selectedLine.value) + ' 线路设置' : ({ connection: '连接爱快', lines: '出口线路', rules: '分流规则', ipv6: 'IPv6 设置', notifications: '通知设置', timing: '检测与回切' }[panelKind.value]))
const modalWidth = computed(() => {
  if (!mdAndUp.value) return undefined
  if (modalKind.value === 'selection') return '36rem'
  if (modalKind.value === 'config') {
    if (panelKind.value === 'lines' && selectedLine.value) return '32rem'
    if (panelKind.value === 'connection') return '40rem'
    if (panelKind.value === 'timing') return '36rem'
  }
  return '50rem'
})
const modalIcon = computed(() => modalKind.value === 'selection' ? 'mdi-swap-horizontal' : modalKind.value === 'health' ? 'mdi-heart-pulse' : ({ connection: 'mdi-router-wireless', lines: 'mdi-transit-connection-variant', rules: 'mdi-source-branch', ipv6: 'mdi-ip-network-outline', notifications: 'mdi-bell-outline', timing: 'mdi-timer-outline' }[panelKind.value]))
const modalDescription = computed(() => modalKind.value === 'selection' ? '设置自动切换，或固定使用一条线路。' : modalKind.value === 'health' ? '最近一次检测到的线路状态和地址。' : ({ connection: '填写连接爱快需要的地址和账号。', lines: '选择主线路、备用线路和默认出口。', rules: '选择故障转移时需要调整的分流规则。', ipv6: '设置线路和内网使用的 IPv6 配置。', notifications: '设置通知方式和需要通知的事件。', timing: '设置线路检测频率和切换等待时间。' }[panelKind.value]))
const selectionChoices = computed(() => [
  { value: 'auto', title: '自动选择', detail: '主线路优先，故障时切换，恢复后回切。' },
  { value: 'primary', title: '主线路', detail: lineLabel(status.value?.primaryName ?? '') },
  { value: 'backup', title: '备用线路', detail: lineLabel(status.value?.backupName ?? '') },
])
const selectedLineMode = computed(() => selectedLine.value === status.value?.primaryName ? 'primary' : selectedLine.value === status.value?.backupName ? 'backup' : null)
let selectionVersion = 0
const { username, password, remember: rememberPassword, ready: loginStorageReady, storageError: loginStorageError, restore: restoreRememberedLogin, rememberSuccessfulLogin } = useRememberedLogin()
const passwordVisible = ref(false)
const account = computed(() => session.value?.username ?? '')
const { preference, scheme, label: themeLabel, choices: themeChoices } = useAppearance()
const { unread, isRead, markRead, markAllRead } = useNotificationReadState(notices, account)

watch(authenticated, value => { if (!value) { password.value = ''; closeOverlays(); dialogOpen.value = false; if (rememberPassword.value) void restoreRememberedLogin(true) } })
watch(restarting, value => { if (value) dialogOpen.value = false })
watch(notificationOpen, value => { if (value) activePopover.value = '' })
function closeOverlays() { activePopover.value = ''; notificationOpen.value = false; mobileNavigation.value = false }
function navigate(target: Section) { changePage(target); closeOverlays() }
async function showConfigurationDialog() {
  closeOverlays()
  dialogOpen.value = true
  await nextTick()
}
function closeConfigurationDialog() {
  dialogOpen.value = false
  selectionVersion += 1
  selectionLoading.value = false
  selectionSaving.value = false
}
function openSettings(kind: CardPanelKind) {
  modalKind.value = 'config'
  panelKind.value = kind
  selectedLine.value = ''
  void showConfigurationDialog()
}
function openLineSettings(name: string) {
  modalKind.value = 'config'
  panelKind.value = 'lines'
  selectedLine.value = name
  void showConfigurationDialog()
}
function openHealth() {
  modalKind.value = 'health'
  selectedLine.value = ''
  void showConfigurationDialog()
}
function selectionFailure(reason: unknown) {
  const data = object(reason), payload = object(data.payload)
  return typeof payload.message === 'string' ? payload.message : '无法修改出口选择，请重试。'
}
async function openSelection(preferred?: 'primary' | 'backup') {
  const version = ++selectionVersion
  modalKind.value = 'selection'
  selectionLoaded.value = false
  selectionLoading.value = true
  selectionError.value = ''
  await showConfigurationDialog()
  try {
    const result = object(await authenticatedRequest('/wan/selection'))
    if (version !== selectionVersion || !authenticated.value) return
    if (!['auto', 'primary', 'backup'].includes(String(result.mode))) throw new Error('出口选择数据不可用')
    selectionMode.value = preferred ?? result.mode as 'auto' | 'primary' | 'backup'
    selectionLoaded.value = true
  } catch (reason: unknown) { if (version === selectionVersion) selectionError.value = selectionFailure(reason) }
  finally { if (version === selectionVersion) selectionLoading.value = false }
}
async function saveSelection() {
  if (!selectionLoaded.value || selectionSaving.value) return
  const version = selectionVersion
  selectionSaving.value = true
  selectionError.value = ''
  try {
    const result = object(await authenticatedRequest('/wan/selection', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ mode: selectionMode.value }) }))
    if (version !== selectionVersion || !authenticated.value) return
    if (result.mode !== selectionMode.value) throw new Error('出口选择保存失败')
    await refresh()
    dialogOpen.value = false
  } catch (reason: unknown) { if (version === selectionVersion) selectionError.value = selectionFailure(reason) }
  finally { if (version === selectionVersion) selectionSaving.value = false }
}
function openLogRecords() { drawerView.value = 'notifications'; notificationOpen.value = true }
function openFullLogs() { logView.value = drawerView.value; navigate('logs') }
function openNotifications() { drawerView.value = 'realtime'; notificationOpen.value = true }
function confirmRestart() { activePopover.value = ''; restartConfirmationOpen.value = true }
function restartConfirmed() { restartConfirmationOpen.value = false; void restartService() }
async function signOut() { closeOverlays(); await logout() }
function escape(event: KeyboardEvent) { if (event.key === 'Escape') closeOverlays() }
onMounted(() => document.addEventListener('keydown', escape))
onUnmounted(() => document.removeEventListener('keydown', escape))

async function rememberBrowserPassword(account: string, secret: string) {
  type PasswordCredentialFactory = new (data: { id: string; password: string; name: string }) => Credential
  const constructor = (window as Window & { PasswordCredential?: PasswordCredentialFactory }).PasswordCredential
  if (!window.isSecureContext || !constructor || !navigator.credentials?.store) return
  try { await navigator.credentials.store(new constructor({ id: account, password: secret, name: account })) }
  catch { /* The browser controls whether credentials can be saved. */ }
}
async function submitLogin() {
  if (!loginStorageReady.value || !username.value || !password.value) return
  const value = password.value
  passwordVisible.value = false
  const success = await login(username.value, value, rememberPassword.value)
  if (success) {
    await rememberSuccessfulLogin(username.value, value)
    if (rememberPassword.value) void rememberBrowserPassword(username.value, value)
    password.value = ''
  }
}
const activeName = computed(() => status.value?.active === 'backup' ? status.value.backupName : status.value?.primaryName ?? 'wan1')
const activeLabel = computed(() => status.value?.active === 'backup' ? '备用线路' : '主线路')
const activeIpv4 = computed(() => status.value?.lines.find(line => line.name === activeName.value)?.address ?? '')
const activeIpv6 = computed(() => status.value?.ipv6.find(line => line.name === activeName.value)?.address ?? '')
const healthyLines = computed(() => status.value?.lines.filter(line => line.health === 'up').length ?? 0)
const healthyPercentage = computed(() => status.value?.lines.length ? healthyLines.value / status.value.lines.length * 100 : 0)
const activeHealth = computed(() => status.value?.lines.find(line => line.name === activeName.value)?.health ?? 'unknown')
const ikuaiInfo = computed(() => status.value?.ikuaiInfo ?? null)
function ikuaiValue(key: 'hostname' | 'version' | 'model') { return ikuaiInfo.value?.[key] ?? '等待获取' }
function percentage(value: number) { return value.toLocaleString('zh-CN', { maximumFractionDigits: 1 }) + '%' }
const ikuaiUptime = computed(() => {
  const value = ikuaiInfo.value?.uptimeSeconds
  if (typeof value !== 'number' || !Number.isFinite(value) || value < 0) return '等待获取'
  const seconds = Math.floor(value), days = Math.floor(seconds / 86400), hours = Math.floor(seconds % 86400 / 3600), minutes = Math.floor(seconds % 3600 / 60)
  return (days ? days + ' 天 ' : '') + (hours || days ? hours + ' 小时 ' : '') + (minutes || hours || days ? minutes + ' 分 ' : '') + seconds % 60 + ' 秒'
})
const quickActions = [
  { label: '线路', title: '配置出口线路', icon: 'mdi-lan', color: 'primary', run: () => openSettings('lines') },
  { label: '规则', title: '配置分流规则', icon: 'mdi-source-branch', color: 'success', run: () => openSettings('rules') },
  { label: 'IPv6', title: '配置 IPv6', icon: 'mdi-ip-network-outline', color: 'warning', run: () => openSettings('ipv6') },
  { label: '通知', title: '配置通知渠道', icon: 'mdi-bell-outline', color: 'info', run: () => openSettings('notifications') },
  { label: '日志', title: '查看实时日志', icon: 'mdi-console-line', color: 'secondary', run: () => openNotifications() },
  { label: '检测', title: '配置检测与回切', icon: 'mdi-timer-outline', color: 'error', run: () => openSettings('timing') },
  { label: '连接', title: '连接爱快', icon: 'mdi-router-wireless', color: 'secondary', run: () => openSettings('connection') },
  { label: '重启', title: '重启服务', icon: 'mdi-restart', color: 'primary', run: () => confirmRestart() },
]
const problems = computed(() => [...new Set([...(status.value?.blockers ?? []), status.value?.lastError, status.value?.ipv6Error].filter((text): text is string => Boolean(text)))])
const ipv6Label = computed(() => ({ disabled: '未开启', checking: '检测中', configuration_verified: '配置已确认', operational_verified: 'IPv6 正常', degraded: '等待恢复', required_missing: '等待配置', awaiting_configuration: '等待配置' }[status.value?.ipv6Capability ?? ''] ?? '等待检测'))
const channels = computed(() => {
  const result = new Map<string, { tag: string; provider: string; latestSuccess: boolean | null }>()
  for (const notice of notices.value) {
    for (const delivery of notice.deliveries) if (!result.has(delivery.tag)) result.set(delivery.tag, { tag: delivery.tag, provider: delivery.provider, latestSuccess: delivery.success })
    for (const tag of notice.pending) if (!result.has(tag)) result.set(tag, { tag, provider: '', latestSuccess: null })
  }
  return [...result.values()]
})
function lineCarrier(name: string) {
  const carrier = status.value?.interfaceNames[name]?.trim() ?? ''
  return carrier.toUpperCase() === name.toUpperCase() ? '' : carrier
}
function lineLabel(name: string) { return name ? name.toUpperCase() + (lineCarrier(name) ? ' ' + lineCarrier(name) : '') : '等待获取' }
function issueLabel(text: string) {
  if (/no managed.*rules|add actual.*rule/i.test(text)) return '尚未配置主备规则，请从爱快读取并选择规则。'
  if (/ipv6|dhcp|prefix|delegat|\bpd\b/i.test(text)) return 'IPv6 地址或前缀尚未满足当前出口要求，请核对线路的 IPv6 与 PD 状态。'
  if (/auth|credential|login|token|unauthor/i.test(text)) return '无法验证爱快连接，请核对连接账号密码。'
  if (/conflict|snapshot|changed|identity|fingerprint|absent|unknown.*field/i.test(text)) return '当前分流规则与服务配置不一致，自动应用已暂缓。'
  if (/network|connection|timeout|request|http|unreachable/i.test(text)) return '爱快连接或线路检查暂未成功，请确认网络连接。'
  return '故障转移尚未完成，请查看日志并核对配置。'
}
function resourceLabel(resource: string) { return ({ five_tuple_rules: '防回环', load_balance_rules: '运营商分流', domain_rules: '域名分流', domain_rule_state: '域名分流', ipv6_wan: 'IPv6 出口', ipv6_lan: 'LAN 前缀' }[resource] ?? '分流规则') }
function fieldLabel(value: string) { return value === 'yes' ? '启用' : value === 'no' ? '停用' : value.toUpperCase() || '未设置' }
</script>

<template>
  <VApp>
    <div v-if="booting" class="boot-screen" aria-live="polite">
      <VAvatar color="primary" variant="tonal" rounded="lg" size="56"><Icon name="network" :size="28" /></VAvatar>
      <strong>ikuai-failover</strong>
      <VProgressCircular indeterminate color="primary" :size="26" :width="2" />
      <p>正在连接服务…</p>
    </div>
    <main v-else-if="!authenticated" class="auth-page">
      <VBtn class="auth-theme" icon variant="text" :aria-label="scheme === 'dark' ? '切换浅色主题' : '切换深色主题'" @click="preference = scheme === 'dark' ? 'light' : 'dark'"><Icon :name="scheme === 'dark' ? 'sun' : 'moon'" /></VBtn>
      <div class="auth-wrapper"><VCard class="auth-card" max-width="24rem" rounded="xl" flat>
        <div class="auth-head"><VAvatar color="primary" variant="tonal" rounded="lg" size="56"><Icon name="network" :size="30" /></VAvatar><h1>ikuai-failover</h1><p>欢迎回来</p></div>
        <form class="auth-form" action="/auth/login" method="post" autocomplete="on" @submit.prevent="submitLogin">
          <VAlert v-if="authError" type="error" variant="tonal" class="mb-5">{{ authError }}</VAlert>
          <VAlert v-if="loginStorageError" type="warning" variant="tonal" class="mb-5">{{ loginStorageError }}</VAlert>
          <VProgressLinear v-if="!loginStorageReady" indeterminate color="primary" class="mb-4" />
          <div class="auth-input-field"><VIcon icon="mdi-account-outline" class="auth-input-icon" aria-hidden="true" /><input id="username" v-model="username" name="username" type="text" autocomplete="username" autocapitalize="none" spellcheck="false" enterkeyhint="next" placeholder="账号" aria-label="账号" required maxlength="256" :disabled="authenticating || !loginStorageReady" /></div>
          <div class="auth-input-field auth-input-field--password"><VIcon icon="mdi-lock-outline" class="auth-input-icon" aria-hidden="true" /><input id="password" v-model="password" name="password" :type="passwordVisible ? 'text' : 'password'" autocomplete="current-password" placeholder="密码" aria-label="密码" required maxlength="1024" :disabled="authenticating || !loginStorageReady" /><button class="auth-password-toggle" type="button" :aria-label="passwordVisible ? '隐藏密码' : '显示密码'" :aria-pressed="passwordVisible" @click="passwordVisible = !passwordVisible"><VIcon :icon="passwordVisible ? 'mdi-eye-off-outline' : 'mdi-eye-outline'" :size="20" /></button></div>
          <label class="auth-remember"><input v-model="rememberPassword" name="remember" type="checkbox" :disabled="authenticating || !loginStorageReady" /><span>记住密码</span></label>
          <VBtn type="submit" color="primary" variant="flat" block size="large" prepend-icon="mdi-login" class="auth-submit" :loading="authenticating" :disabled="authenticating || !loginStorageReady || !username || !password">登录</VBtn>
          <p class="auth-note">首次登录请使用配置文件中的网页登录账号。</p>
        </form>
      </VCard></div>
    </main>
    <VLayout v-else class="mp-layout">
      <VNavigationDrawer v-model="drawerVisible" class="mp-navigation" :permanent="mdAndUp" :temporary="!mdAndUp" :rail="mdAndUp && railNavigation" :width="240" :rail-width="80">
        <a class="mp-nav-brand" href="#" @click.prevent="navigate('overview')"><VAvatar color="primary" rounded="lg" size="42"><Icon name="network" :size="24" /></VAvatar><span v-if="!mdAndUp || !railNavigation"><strong>ikuai-failover</strong><small>故障转移控制台</small></span></a>
        <VList nav density="comfortable" class="mp-navigation-list"><VListItem v-for="item in navigation" :key="item.key" :title="item.title" :active="section === item.key" :color="section === item.key ? 'white' : undefined" rounded="pill" @click="navigate(item.key)"><template #prepend><Icon :name="item.icon" :size="21" /></template></VListItem></VList>
        <template #append><p v-if="!mdAndUp || !railNavigation" class="mp-nav-footer">ikuai-failover</p></template>
      </VNavigationDrawer>
      <VAppBar class="mp-appbar" :height="64" flat>
        <VContainer fluid class="mp-toolbar"><div class="mp-toolbar-title"><VBtn icon variant="text" size="small" aria-label="打开导航" @click="mdAndUp ? railNavigation = !railNavigation : mobileNavigation = true"><Icon name="menu" /></VBtn><span v-if="section !== 'settings'">{{ page.title }}</span></div><div class="mp-toolbar-actions">
          <VMenu v-model="themeMenu" location="bottom end" :offset="10"><template #activator="{ props: menuProps }"><VBtn v-bind="menuProps" icon variant="text" size="small" aria-label="切换主题" :title="themeLabel"><Icon :name="scheme === 'dark' ? 'moon' : 'sun'" :size="21" /></VBtn></template><VCard min-width="210"><VCardSubtitle class="pt-4 pb-2">外观主题</VCardSubtitle><VList density="compact"><VListItem v-for="choice in themeChoices" :key="choice.value" :title="choice.label" :active="preference === choice.value" @click="preference = choice.value; themeMenu = false"><template #prepend><Icon :name="choice.icon" :size="18" /></template><template v-if="preference === choice.value" #append><Icon name="check" :size="16" /></template></VListItem></VList></VCard></VMenu>
          <VBadge :model-value="unread > 0" :content="unread > 99 ? '99+' : unread" color="error" :offset-x="4" :offset-y="4"><VBtn icon variant="text" size="small" aria-label="打开通知中心" title="通知中心" @click="openNotifications"><Icon name="bell" :size="22" /></VBtn></VBadge>
          <VMenu v-model="profileMenu" location="bottom end" :offset="8"><template #activator="{ props: menuProps }"><VBtn v-bind="menuProps" icon variant="text" :aria-label="(account || '用户') + '的菜单'"><VAvatar color="primary" variant="tonal" size="36">{{ account.slice(0, 1).toUpperCase() || 'U' }}</VAvatar></VBtn></template><VList class="profile-menu" density="compact" rounded="lg" elevation="8" border><VListItem title="重启服务" @click="confirmRestart"><template #prepend><Icon name="refresh" :size="21" /></template></VListItem><VListItem title="退出登录" @click="signOut"><template #prepend><Icon name="logout" :size="21" /></template></VListItem></VList></VMenu>
        </div></VContainer>
      </VAppBar>
      <VMain class="mp-main"><VContainer fluid class="mp-page-container">
        <VAlert v-if="loginStorageError" type="warning" variant="tonal" class="mb-5">{{ loginStorageError }}</VAlert>
        <VAlert v-if="statusError && section === 'overview'" type="error" variant="tonal" class="mb-5">{{ statusError }}</VAlert>
        <VAlert v-if="status?.setupRequired && section === 'overview'" type="info" variant="tonal" class="mb-5"><div class="setup-alert-content"><div><strong>请先连接爱快</strong><p>填写账号密码，读取接口和规则后保存。</p></div><VBtn color="primary" variant="flat" @click="openSettings('connection')">连接爱快</VBtn></div></VAlert>
        <template v-if="section === 'overview'">
          <VRow v-if="!status && loading"><VCol v-for="item in 3" :key="item" cols="12" md="4"><VSkeletonLoader type="article" /></VCol></VRow>
          <VCard v-else-if="!status"><VCardText class="dashboard-empty"><Icon name="network" :size="36" /><h2 class="text-h6">等待网络状态</h2><p>服务尚未返回线路检测结果。</p><VBtn color="primary" variant="tonal" :disabled="loading" @click="refresh">重新获取</VBtn></VCardText></VCard>
          <template v-if="status">
            <VRow class="overview-top">
              <VCol cols="12" md="4" class="overview-top-column">
                <VCard class="overview-current interactive-card mb-4" role="button" tabindex="0" aria-label="选择当前出口" @click="openSelection()" @keydown.enter.self.prevent="openSelection()" @keydown.space.self.prevent="openSelection()">
                  <VCardItem title="当前出口"><template #append><VAvatar color="primary" variant="tonal" rounded size="48"><VIcon icon="mdi-router-network" size="30" /></VAvatar></template></VCardItem>
                  <VCardText class="overview-current-body"><div class="overview-egress-name">{{ activeName.toUpperCase() }}<span>{{ lineCarrier(activeName) }}</span></div><div class="overview-current-caption"><span>{{ activeLabel }}</span><VChip :color="activeHealth === 'up' ? 'success' : activeHealth === 'down' ? 'error' : 'default'" size="x-small" variant="tonal">{{ healthLabel(activeHealth) }}</VChip></div><VProgressLinear v-if="status.wanEnabled && status.lines.length" :model-value="healthyPercentage" color="primary" height="5" rounded class="mt-4" /><p class="overview-caption mt-2">{{ status.wanEnabled ? healthyLines + ' / ' + status.lines.length + ' 条线路正常' : '线路监测未开启' }}</p></VCardText>
                </VCard>
                <VCard class="overview-line-health interactive-card" role="button" tabindex="0" aria-label="查看线路健康" @click="openHealth" @keydown.enter.self.prevent="openHealth" @keydown.space.self.prevent="openHealth"><VCardItem title="线路状态" /><VCardText class="pt-0"><div v-for="line in status.lines" :key="line.name" class="overview-health-line"><div><strong>{{ lineLabel(line.name) }}</strong><code>{{ line.address || '等待获取地址' }}</code></div><VChip :color="line.health === 'up' ? 'success' : line.health === 'down' ? 'error' : 'default'" size="x-small" variant="tonal">{{ healthLabel(line.health) }}</VChip></div><div class="overview-health-foot"><span>上次切换</span><span>{{ timeLabel(status.lastTransition) }}</span></div></VCardText></VCard>
              </VCol>
              <VCol cols="12" md="8" class="overview-top-column">
                <VCard class="overview-statistics mb-4"><VCardItem title="网络状态统计" /><VCardText class="pt-0"><VRow dense class="overview-statistic-row"><VCol cols="6" sm="3" md="6" lg="3"><div class="overview-statistic interactive-card" role="button" tabindex="0" aria-label="查看线路健康" @click="openHealth" @keydown.enter.self.prevent="openHealth" @keydown.space.self.prevent="openHealth"><VAvatar color="primary" rounded="circle" size="38"><VIcon icon="mdi-lan" :size="22" /></VAvatar><div><span>线路</span><strong>{{ healthyLines }}<small>/ {{ status.lines.length }}</small></strong><p class="text-success">已正常</p></div></div></VCol><VCol cols="6" sm="3" md="6" lg="3"><div class="overview-statistic interactive-card" role="button" tabindex="0" aria-label="配置分流规则" @click="openSettings('rules')" @keydown.enter.self.prevent="openSettings('rules')" @keydown.space.self.prevent="openSettings('rules')"><VAvatar color="success" rounded="circle" size="38"><VIcon icon="mdi-source-branch" :size="22" /></VAvatar><div><span>策略</span><strong>{{ status.changes.length }}</strong><p>{{ problems.length ? problems.length + ' 项需关注' : '待调整规则' }}</p></div></div></VCol><VCol cols="6" sm="3" md="6" lg="3"><div class="overview-statistic interactive-card" role="button" tabindex="0" aria-label="查看通知记录" @click="openLogRecords" @keydown.enter.self.prevent="openLogRecords" @keydown.space.self.prevent="openLogRecords"><VAvatar color="warning" rounded="circle" size="38"><VIcon icon="mdi-send-check-outline" :size="22" /></VAvatar><div><span>通知</span><strong>{{ status.notifications.delivered }}</strong><p>成功发送</p></div></div></VCol><VCol cols="6" sm="3" md="6" lg="3"><div class="overview-statistic interactive-card" role="button" tabindex="0" aria-label="查看等待发送的通知" @click="openLogRecords" @keydown.enter.self.prevent="openLogRecords" @keydown.space.self.prevent="openLogRecords"><VAvatar color="info" rounded="circle" size="38"><VIcon icon="mdi-tray-arrow-up" :size="22" /></VAvatar><div><span>队列</span><strong>{{ status.notifications.queued }}</strong><p>等待发送</p></div></div></VCol></VRow><VDivider class="my-4" /><div class="overview-network"><div class="dashboard-address-row interactive-card" role="button" tabindex="0" aria-label="配置出口线路" @click="openSettings('lines')" @keydown.enter.prevent="openSettings('lines')" @keydown.space.prevent="openSettings('lines')"><span>IPv4</span><code tabindex="0" class="dashboard-address-value" aria-label="当前 IPv4 地址">{{ activeIpv4 || '等待获取' }}</code></div><div class="dashboard-address-row interactive-card" role="button" tabindex="0" aria-label="配置 IPv6" @click="openSettings('ipv6')" @keydown.enter.prevent="openSettings('ipv6')" @keydown.space.prevent="openSettings('ipv6')"><span>IPv6</span><code tabindex="0" class="dashboard-address-value" aria-label="当前 IPv6 地址">{{ activeIpv6 || (status.ipv6Capability === 'disabled' ? '未开启' : '等待获取') }}</code></div></div></VCardText></VCard>
                <VCard class="overview-quick-card mb-4"><VCardItem title="快捷操作" /><VCardText class="pt-0"><VRow no-gutters class="overview-quick-actions"><VCol v-for="action in quickActions" :key="action.label" class="overview-quick-col"><VBtn variant="text" color="default" block height="auto" class="overview-quick-button" :aria-label="action.title" @click="action.run"><span class="overview-quick-content"><VAvatar :color="action.color" variant="tonal" rounded size="34"><VIcon :icon="action.icon" :size="21" /></VAvatar><span>{{ action.label }}</span></span></VBtn></VCol></VRow></VCardText></VCard>
                <VCard class="overview-service interactive-card" role="button" tabindex="0" aria-label="配置爱快连接" @click="openSettings('connection')" @keydown.enter.self.prevent="openSettings('connection')" @keydown.space.self.prevent="openSettings('connection')"><VCardItem title="爱快信息"><template v-if="ikuaiInfo?.stale" #append><VChip size="x-small" variant="tonal">上次记录</VChip></template></VCardItem><VCardText class="pt-0"><VRow dense><VCol cols="12" sm="4"><div class="overview-service-field"><span>系统名称</span><strong>{{ ikuaiValue('hostname') }}</strong></div></VCol><VCol cols="12" sm="4"><div class="overview-service-field"><span>爱快版本</span><strong>{{ ikuaiValue('version') }}</strong></div></VCol><VCol cols="12" sm="4"><div class="overview-service-field"><span>运行时间</span><strong>{{ ikuaiUptime }}</strong></div></VCol></VRow><div v-if="typeof ikuaiInfo?.cpuPercent === 'number' || typeof ikuaiInfo?.memoryPercent === 'number'" class="ikuai-resource-row"><span v-if="typeof ikuaiInfo?.cpuPercent === 'number'"><VIcon icon="mdi-chip" size="15" />CPU{{ ikuaiInfo.usageSampledAt !== undefined ? ' 采样' : '' }} {{ percentage(ikuaiInfo.cpuPercent) }}</span><span v-if="typeof ikuaiInfo?.memoryPercent === 'number'"><VIcon icon="mdi-memory" size="15" />内存{{ ikuaiInfo.usageSampledAt !== undefined ? ' 采样' : '' }} {{ percentage(ikuaiInfo.memoryPercent) }}</span></div></VCardText></VCard>
              </VCol>
            </VRow>
            <VCard class="overview-main-card"><VCardItem title="出口线路"><template #append><div class="dashboard-actions"><span class="text-caption">{{ paused ? '刷新已暂停' : '每 5 秒刷新' }}</span><VBtn icon variant="text" size="small" :aria-label="paused ? '恢复自动刷新' : '暂停自动刷新'" @click="paused = !paused"><Icon :name="paused ? 'play' : 'pause'" :size="18" /></VBtn><VBtn icon variant="text" size="small" :disabled="loading" aria-label="刷新线路" @click="refresh"><Icon name="refresh" :size="18" :class="{ rotating: loading }" /></VBtn></div></template></VCardItem><VCardText class="pt-0"><VRow><VCol v-for="line in status.lines" :key="line.name" cols="12" md="4"><VCard variant="tonal" class="dashboard-line interactive-card" :class="{ 'dashboard-line--active': line.name === activeName }" role="button" tabindex="0" :aria-label="lineLabel(line.name) + '线路设置'" @click="openLineSettings(line.name)" @keydown.enter.self.prevent="openLineSettings(line.name)" @keydown.space.self.prevent="openLineSettings(line.name)"><VCardItem><template #title><h3 class="dashboard-line-title">{{ line.name.toUpperCase() }}<span v-if="lineCarrier(line.name)">{{ lineCarrier(line.name) }}</span></h3></template><VCardSubtitle>{{ line.name === status.primaryName ? '主线路' : line.name === status.backupName ? '备用线路' : line.name === status.preservedDefault ? 'OpenWrt 代理出口' : '网络接口' }}</VCardSubtitle><template #append><VChip :color="line.health === 'up' ? 'success' : line.health === 'down' ? 'error' : 'default'" size="small" variant="tonal">{{ healthLabel(line.health) }}</VChip></template></VCardItem><VCardText><div class="dashboard-address-row"><span>IPv4</span><code class="dashboard-address-value" tabindex="0">{{ line.address || '等待获取' }}</code></div></VCardText><VCardActions class="dashboard-line-footer"><VChip v-if="line.name === activeName" color="primary" size="small" variant="tonal">当前出口</VChip><span v-else class="text-caption">{{ line.name === status.preservedDefault ? '代理出口' : '待命出口' }}</span></VCardActions></VCard></VCol></VRow></VCardText></VCard>
            <VRow class="dashboard-detail-row">
              <VCol cols="12" lg="6"><VCard class="dashboard-panel interactive-card" role="button" tabindex="0" aria-label="配置 IPv6 与 PD 前缀" @click="openSettings('ipv6')" @keydown.enter.self.prevent="openSettings('ipv6')" @keydown.space.self.prevent="openSettings('ipv6')"><VCardItem title="IPv6 与 PD 前缀"><template #append><div class="dashboard-actions"><VChip :color="status.ipv6Capability === 'operational_verified' ? 'success' : status.ipv6Capability === 'degraded' ? 'warning' : 'default'" size="small" variant="tonal">{{ ipv6Label }}</VChip><VBtn icon variant="text" size="small" aria-label="配置 IPv6" @click.stop="openSettings('ipv6')"><Icon name="settings" :size="18" /></VBtn></div></template></VCardItem><VCardText><div v-if="status.ipv6Ready" class="dashboard-evidence"><VChip :color="status.ipv6Ready.address ? 'success' : 'default'" size="small" variant="tonal">出口地址</VChip><VChip :color="status.ipv6Ready.prefix ? 'success' : 'default'" size="small" variant="tonal">PD 前缀</VChip><VChip :color="status.ipv6Ready.lan ? 'success' : 'default'" size="small" variant="tonal">内网前缀</VChip></div><p v-if="!status.ipv6.length" class="dashboard-muted">{{ status.ipv6Capability === 'disabled' ? 'IPv6 监测未开启。' : '暂未读到 IPv6 地址与前缀。' }}</p><dl v-for="line in status.ipv6" :key="line.name + '-' + line.address" class="dashboard-address-list"><div><dt>当前接口</dt><dd>{{ lineLabel(line.name) }}</dd></div><div><dt>IPv6 地址</dt><dd><code class="dashboard-address-value" tabindex="0">{{ line.address || '等待获取' }}</code></dd></div><div><dt>委派前缀</dt><dd><code class="dashboard-address-value" tabindex="0">{{ line.prefix || '等待获取' }}</code></dd></div></dl></VCardText></VCard></VCol>
              <VCol cols="12" lg="6"><VCard class="dashboard-panel interactive-card" role="button" tabindex="0" aria-label="配置故障转移规则" @click="openSettings('rules')" @keydown.enter.self.prevent="openSettings('rules')" @keydown.space.self.prevent="openSettings('rules')"><VCardItem title="故障转移"><template #append><VChip :color="status.applyEnabled ? 'primary' : 'default'" size="small" variant="tonal">{{ status.applyEnabled ? '自动应用' : '观察模式' }}</VChip></template></VCardItem><VCardText><div v-if="problems.length" class="dashboard-issues"><VAlert v-for="(issue, index) in problems" :key="index" type="warning" variant="tonal">{{ issueLabel(issue) }}</VAlert></div><div v-else class="dashboard-policy-good"><VAvatar color="success" variant="tonal" size="42"><Icon name="check" :size="23" /></VAvatar><div><strong>本轮未报告异常</strong><p>{{ status.changes.length ? '检测到需要调整的分流规则。' : '线路规则与最近一次检查结果一致。' }}</p></div></div><div v-if="status.changes.length" class="dashboard-policy-changes"><div v-for="change in status.changes.slice(0, 4)" :key="change.resource + '-' + change.id"><span>{{ change.tag || resourceLabel(change.resource) }}</span><span class="dashboard-change-values">{{ fieldLabel(change.before) }}<Icon name="arrow" :size="14" /><strong>{{ fieldLabel(change.after) }}</strong></span></div><p v-if="status.changes.length > 4" class="text-caption">另有 {{ status.changes.length - 4 }} 项变化</p></div></VCardText><VCardActions class="dashboard-panel-footer"><span class="text-caption">上次切换：{{ timeLabel(status.lastTransition) }}</span><VSpacer /><VBtn color="primary" variant="text" @click.stop="openSettings('rules')">配置规则</VBtn></VCardActions></VCard></VCol>
            </VRow>
            <VCard class="dashboard-notifications interactive-card" role="button" tabindex="0" aria-label="配置通知渠道" @click="openSettings('notifications')" @keydown.enter.self.prevent="openSettings('notifications')" @keydown.space.self.prevent="openSettings('notifications')"><VCardItem title="通知发送" :subtitle="status.notifications.enabled ? '查看通知发送结果。' : '通知推送未开启，运行事件继续记录。'"><template #append><div class="dashboard-actions"><VBtn icon variant="text" size="small" aria-label="配置通知渠道" @click.stop="openSettings('notifications')"><Icon name="settings" :size="18" /></VBtn><VBtn color="primary" variant="text" @click.stop="openLogRecords">通知记录</VBtn></div></template></VCardItem><VCardText><VRow class="dashboard-notification-counts"><VCol v-for="counter in [{ value: status.notifications.history, label: '事件记录' }, { value: status.notifications.delivered, label: '发送成功' }, { value: status.notifications.queued, label: '等待发送' }, { value: status.notifications.failed, label: '发送失败' }]" :key="counter.label" cols="6" sm="3"><strong>{{ counter.value }}</strong><span>{{ counter.label }}</span></VCol></VRow><VRow v-if="channels.length" class="mt-4"><VCol v-for="channel in channels.slice(0, 6)" :key="channel.tag" cols="12" sm="6" md="4"><div class="dashboard-channel"><VAvatar size="36" rounded="lg" class="dashboard-channel-avatar"><VImg :src="notificationProviderIcon(channel.provider)" width="32" height="32" contain aria-hidden="true" /></VAvatar><div class="dashboard-channel-copy"><strong>{{ channel.provider ? providerLabel(channel.provider) : channel.tag }}</strong><span>{{ channel.provider ? channel.tag : '等待首次发送' }}</span></div><VChip :color="channel.latestSuccess === null ? 'default' : channel.latestSuccess ? 'success' : 'error'" size="x-small" variant="tonal">{{ channel.latestSuccess === null ? '等待' : channel.latestSuccess ? '成功' : '失败' }}</VChip></div></VCol></VRow><p v-else class="dashboard-muted mt-4">尚无渠道发送记录。</p></VCardText></VCard>
          </template>
        </template>
        <LogsCenter v-else-if="section === 'logs'" v-model="logView" :request="authenticatedRequest" :notices="notices" :loading="loading" :error="historyError" :unread="unread" :is-read="isRead" :active="!notificationOpen && !restarting && !dialogOpen" @read="markRead" @read-all="markAllRead" @refresh="refresh" />
        <ConfigEditor v-else :initial-group="settingsGroup" :request="authenticatedRequest" @update:group="selectSettings" @restart="restartService" />
        <VAlert v-if="restartError" type="error" variant="tonal" class="mt-5">{{ restartError }}</VAlert>
        <footer v-if="section !== 'logs'" class="dashboard-footer"><span><Icon name="clock" :size="14" />{{ updatedAt ? '状态更新于 ' + timeLabel(updatedAt) : '等待首次状态更新' }}</span><span>自动切换 · 恢复自动回切</span></footer>
      </VContainer></VMain>

      <VDialog v-model="notificationOpen" :aria-label="drawerTitle" :max-width="mdAndUp ? '72rem' : undefined" :fullscreen="!mdAndUp" scrollable class="mp-logs-dialog"><VCard class="dashboard-dialog-card dashboard-log-dialog-card" :height="mdAndUp ? 'min(82dvh,52rem)' : '100dvh'" :max-height="mdAndUp ? 'calc(100dvh - 2rem)' : '100dvh'" :style="{ flex: '0 0 auto' }" :class="{ 'dashboard-dialog-card--fullscreen': !mdAndUp }" :rounded="!mdAndUp ? 0 : undefined"><VCardItem class="dashboard-dialog-header dashboard-log-dialog-header"><VCardTitle>{{ drawerTitle }}</VCardTitle><template #append><VBtn icon variant="text" size="small" aria-label="在页面中打开日志" title="在页面中打开" @click="openFullLogs"><VIcon icon="mdi-open-in-new" :size="20" /></VBtn><VBtn icon variant="text" size="small" aria-label="关闭日志" @click="notificationOpen = false"><Icon name="close" :size="22" /></VBtn></template></VCardItem><div class="dashboard-dialog-body dashboard-dialog-body--logs"><LogsCenter v-if="notificationOpen" v-model="drawerView" compact :request="authenticatedRequest" :notices="notices" :loading="loading" :error="historyError" :unread="unread" :is-read="isRead" @read="markRead" @read-all="markAllRead" @refresh="refresh" /></div></VCard></VDialog>
      <VDialog v-model="dialogOpen" :aria-label="modalTitle" :max-width="modalWidth" :fullscreen="!mdAndUp" :persistent="selectionSaving || (modalKind === 'config' && Boolean(cardPanel?.saving))" scrollable class="mp-card-settings-dialog" @after-leave="closeConfigurationDialog">
        <VCard class="dashboard-dialog-card" :class="{ 'dashboard-dialog-card--fullscreen': !mdAndUp }" :rounded="!mdAndUp ? 0 : undefined">
          <VCardItem class="dashboard-dialog-header"><template #prepend><VAvatar color="primary" variant="tonal" rounded size="40"><VIcon :icon="modalIcon" :size="22" /></VAvatar></template><VCardTitle>{{ modalTitle }}</VCardTitle><VCardSubtitle>{{ modalDescription }}</VCardSubtitle><template #append><VBtn icon variant="text" aria-label="关闭配置" :disabled="selectionSaving || Boolean(cardPanel?.saving)" @click="dialogOpen = false"><Icon name="close" :size="22" /></VBtn></template></VCardItem>
          <div class="dashboard-dialog-body">
            <template v-if="dialogOpen">
              <div v-if="modalKind === 'selection'" class="egress-selection-panel"><VAlert v-if="selectionError" type="error" variant="tonal" class="mb-5">{{ selectionError }}</VAlert><VProgressLinear v-if="selectionLoading" indeterminate color="primary" class="mb-5" /><VRadioGroup v-model="selectionMode" class="egress-choice-group" :disabled="selectionLoading || selectionSaving || !selectionLoaded" hide-details><VRadio v-for="choice in selectionChoices" :key="choice.value" :value="choice.value" class="egress-choice"><template #label><div class="egress-choice-copy"><strong>{{ choice.title }}</strong><span>{{ choice.detail }}</span></div></template></VRadio></VRadioGroup></div>
              <div v-else-if="modalKind === 'health'" class="health-panel"><VRow><VCol v-for="line in status?.lines ?? []" :key="line.name" cols="12"><VCard variant="outlined" class="health-line"><VCardItem :title="lineLabel(line.name)"><template #append><VChip :color="line.health === 'up' ? 'success' : line.health === 'down' ? 'error' : 'default'" size="small" variant="tonal">{{ healthLabel(line.health) }}</VChip></template></VCardItem><VCardText><div class="dashboard-address-row"><span>IPv4</span><code tabindex="0" class="dashboard-address-value">{{ line.address || '等待获取' }}</code></div><div class="dashboard-address-row"><span>IPv6</span><code tabindex="0" class="dashboard-address-value">{{ status?.ipv6.find(item => item.name === line.name)?.address || '暂无地址' }}</code></div></VCardText><VCardActions><VSpacer /><VBtn color="primary" variant="text" @click="openLineSettings(line.name)">线路设置</VBtn></VCardActions></VCard></VCol></VRow></div>
              <CardSettingsPanel v-else :key="panelKind + ':' + selectedLine" ref="cardPanel" :kind="panelKind" :line="selectedLine || undefined" :request="authenticatedRequest" @saved="refresh" @restart="restartService" />
            </template>
          </div>
          <VCardActions class="dashboard-dialog-footer">
            <template v-if="modalKind === 'selection'"><VSpacer /><VBtn variant="text" :disabled="selectionSaving" @click="dialogOpen = false">取消</VBtn><VBtn color="primary" variant="flat" prepend-icon="mdi-check" :loading="selectionSaving" :disabled="selectionLoading || selectionSaving || !selectionLoaded" @click="saveSelection">应用选择</VBtn></template>
            <template v-else-if="modalKind === 'health'"><VBtn variant="text" :loading="loading" prepend-icon="mdi-refresh" @click="refresh">刷新状态</VBtn><VSpacer /><VBtn color="primary" variant="flat" @click="openSettings('timing')">检测设置</VBtn></template>
            <template v-else><span v-if="cardPanel?.dirty" class="dashboard-save-state">有未保存的修改</span><VSpacer /><VBtn v-if="panelKind === 'lines' && selectedLineMode" color="primary" variant="text" :disabled="Boolean(cardPanel?.saving)" @click="openSelection(selectedLineMode)">使用此线路</VBtn><VBtn color="primary" variant="flat" prepend-icon="mdi-content-save" :loading="cardPanel?.saving" :disabled="!cardPanel?.canSave" @click="cardPanel?.save()">保存</VBtn></template>
          </VCardActions>
        </VCard>
      </VDialog>
      <VDialog v-model="restartConfirmationOpen" aria-label="重启服务" max-width="28rem"><VCard><VCardItem title="重启服务？" /><VCardText>控制台将短暂断开，服务恢复后页面会自动重新连接。</VCardText><VCardActions class="dashboard-dialog-footer"><VSpacer /><VBtn variant="text" @click="restartConfirmationOpen = false">取消</VBtn><VBtn color="primary" variant="flat" @click="restartConfirmed">立即重启</VBtn></VCardActions></VCard></VDialog>
      <VDialog :model-value="restarting" persistent max-width="28rem" aria-label="服务正在重启"><VCard><VCardText class="text-center"><VProgressCircular indeterminate color="primary" class="mb-4" /><h2 class="text-h6 mb-3">服务正在重启</h2><p>正在等待服务恢复，页面将自动重新连接。</p></VCardText></VCard></VDialog>
    </VLayout>
  </VApp>
</template>

<style scoped>
.boot-screen { min-block-size: 100dvh; display: flex; flex-direction: column; align-items: center; justify-content: center; gap: 18px; color: rgba(var(--v-theme-on-surface), .6); }
.boot-screen strong { font-size: 1.25rem; color: rgb(var(--v-theme-on-surface)); }
/* MP dialog structure: fixed card header/actions, one scrolling body. */
.dashboard-dialog-card {
  display: flex;
  flex-direction: column;
  overflow: hidden;
  max-block-size: calc(100dvh - 2rem);
}
.dashboard-dialog-card--fullscreen { block-size: 100dvh; max-block-size: 100dvh; }
.dashboard-dialog-header { flex: 0 0 auto; border-block-end: 1px solid rgba(var(--v-theme-on-surface), .08); }
.dashboard-dialog-header :deep(.v-card-item__content) { min-inline-size: 0; }
.dashboard-dialog-header :deep(.v-card-title) { white-space: normal; line-height: 1.5; }
.dashboard-dialog-header :deep(.v-card-subtitle) { white-space: normal; line-height: 1.6; margin-block-start: 4px; }
.dashboard-dialog-header :deep(.v-card-item__prepend) { padding-inline-end: 12px; }
.dashboard-dialog-body { flex: 1 1 auto; min-block-size: 0; overflow-y: auto; overscroll-behavior: contain; padding: 20px; }
.dashboard-dialog-body--logs { display: flex; flex-direction: column; padding: 0; overflow: hidden; }
.dashboard-dialog-footer { flex: 0 0 auto; display: flex; flex-wrap: wrap; gap: 8px; border-block-start: 1px solid rgba(var(--v-theme-on-surface), .08); padding: 14px 16px; background: rgb(var(--v-theme-surface)); }
.dashboard-save-state { color: rgba(var(--v-theme-on-surface), .6); font-size: .8125rem; }
.egress-choice-group { inline-size: 100%; }
.egress-choice-group :deep(.v-selection-control-group) { display: flex; flex-direction: column; align-items: stretch; gap: 12px; }
.egress-choice { inline-size: 100%; padding: 12px; border: 1px solid rgba(var(--v-theme-on-surface), .16); border-radius: 12px; }
.egress-choice.v-selection-control--dirty { border-color: rgb(var(--v-theme-primary)); background: rgba(var(--v-theme-primary), .04); }
.egress-choice :deep(.v-selection-control__wrapper) { flex: 0 0 auto; }
.egress-choice :deep(.v-label) { flex: 1 1 auto; min-inline-size: 0; white-space: normal; }
.egress-choice-copy { display: flex; flex-direction: column; gap: 5px; padding-inline-start: 6px; line-height: 1.6; min-inline-size: 0; }
.egress-choice-copy strong { font-size: 1rem; font-weight: 500; color: rgb(var(--v-theme-on-surface)); }
.egress-choice-copy span { font-size: .875rem; color: rgba(var(--v-theme-on-surface), .6); overflow-wrap: anywhere; }
.health-line { min-inline-size: 0; }
.health-line :deep(.v-card-title) { white-space: normal; font-size: 1rem; }

.dashboard-header, .dashboard-section-heading { display: flex; align-items: center; justify-content: space-between; gap: 20px; margin-block-end: 24px; }
.dashboard-header p, .dashboard-section-heading p { margin-block-start: 6px; color: rgba(var(--v-theme-on-surface), .6); font-size: .875rem; line-height: 1.6; }
.setup-alert-content { display: flex; align-items: center; justify-content: space-between; gap: 16px; }
.setup-alert-content p { margin-block-start: 5px; line-height: 1.6; }
.dashboard-empty { display: flex; align-items: center; flex-direction: column; gap: 16px; padding: 44px 20px; text-align: center; color: rgba(var(--v-theme-on-surface), .6); }
.dashboard-metrics { margin-block-end: 4px; }
.dashboard-stat { block-size: 100%; min-inline-size: 0; }
.dashboard-stat-heading { display: flex; align-items: center; justify-content: space-between; gap: 8px; color: rgba(var(--v-theme-on-surface), .6); font-size: .875rem; }
.dashboard-stat-value { margin-block-start: 14px; color: rgb(var(--v-theme-on-surface)); font-size: 1.75rem; line-height: 1.4; font-weight: 500; }
.dashboard-stat-value small { margin-inline-start: 8px; font-size: 1rem; color: rgba(var(--v-theme-on-surface), .6); }
.dashboard-stat-value--text { font-size: 1.25rem; }
.dashboard-stat-detail { margin-block-start: 3px; font-size: .875rem; line-height: 1.6; color: rgba(var(--v-theme-on-surface), .6); min-block-size: 1.4em; }
.dashboard-stat p { margin-block-start: 14px; color: rgba(var(--v-theme-on-surface), .6); font-size: .8125rem; line-height: 1.6; }
.dashboard-network { block-size: 100%; }.dashboard-network-addresses { padding-block-start: 0; }
.dashboard-address-row { display: grid; grid-template-columns: 44px minmax(0, 1fr); gap: 12px; align-items: baseline; min-inline-size: 0; }
.dashboard-address-row + .dashboard-address-row { margin-block-start: 12px; }
.dashboard-address-row > span { color: rgba(var(--v-theme-on-surface), .6); font-size: .875rem; }
.dashboard-address-value { display: block; min-inline-size: 0; max-inline-size: 100%; overflow-x: auto; white-space: nowrap; overflow-wrap: normal; word-break: normal; font: .875rem/1.8 Consolas, 'SFMono-Regular', monospace; color: rgb(var(--v-theme-on-surface)); background: transparent; scrollbar-width: thin; touch-action: pan-x; }
.dashboard-address-value:focus-visible { outline: 2px solid rgba(var(--v-theme-primary), .5); outline-offset: 2px; }
.dashboard-section { margin-block: 20px; }
.dashboard-actions { display: flex; align-items: center; gap: 6px; flex-shrink: 0; }
.dashboard-actions > span { color: rgba(var(--v-theme-on-surface), .6); }
.dashboard-line { block-size: 100%; display: flex; flex-direction: column; min-inline-size: 0; }
.dashboard-line--active { border: 1px solid rgba(var(--v-theme-primary), .3); }
.dashboard-line :deep(.v-card-item__content) { min-inline-size: 0; }
.dashboard-line-title { display: flex; flex-wrap: wrap; align-items: baseline; gap: 8px; font-size: 1rem; line-height: 1.5; font-weight: 500; white-space: normal; }
.dashboard-line-title span { color: rgba(var(--v-theme-on-surface), .6); font-size: .875rem; font-weight: 400; }
.dashboard-line-footer { margin-block-start: auto; border-block-start: 1px solid rgba(var(--v-theme-on-surface), .06); padding: 12px 20px; }
.dashboard-detail-row { margin-block-end: 12px; }
.dashboard-panel { block-size: 100%; display: flex; flex-direction: column; min-inline-size: 0; }
.dashboard-panel :deep(.v-card-title), .dashboard-notifications :deep(.v-card-title) { white-space: normal; }
.dashboard-panel :deep(.v-card-subtitle), .dashboard-notifications :deep(.v-card-subtitle) { white-space: normal; }
.dashboard-evidence { display: flex; flex-wrap: wrap; gap: 8px; margin-block-end: 18px; }
.dashboard-address-list { margin: 0; min-inline-size: 0; }
.dashboard-address-list + .dashboard-address-list { margin-block-start: 20px; padding-block-start: 20px; border-block-start: 1px solid rgba(var(--v-theme-on-surface), .08); }
.dashboard-address-list > div { display: grid; grid-template-columns: 74px minmax(0, 1fr); gap: 12px; align-items: baseline; min-inline-size: 0; margin-block: 10px; }
.dashboard-address-list dt { color: rgba(var(--v-theme-on-surface), .6); font-size: .8125rem; }
.dashboard-address-list dd { margin: 0; min-inline-size: 0; font-size: .875rem; line-height: 1.7; }
.dashboard-policy-good { display: flex; align-items: flex-start; gap: 14px; }
.dashboard-policy-good strong { font-size: .9375rem; font-weight: 500; }
.dashboard-policy-good p { margin-block-start: 6px; color: rgba(var(--v-theme-on-surface), .6); font-size: .875rem; line-height: 1.8; }
.dashboard-issues { display: flex; flex-direction: column; gap: 12px; }
.dashboard-policy-changes { margin-block-start: 20px; }
.dashboard-policy-changes > div { display: flex; align-items: center; justify-content: space-between; gap: 12px; padding-block: 10px; border-block-start: 1px solid rgba(var(--v-theme-on-surface), .08); font-size: .875rem; }
.dashboard-policy-changes > div > span:first-child { min-inline-size: 0; overflow-wrap: anywhere; }
.dashboard-change-values { display: flex; align-items: center; gap: 7px; flex-shrink: 0; color: rgba(var(--v-theme-on-surface), .6); }
.dashboard-change-values strong { color: rgb(var(--v-theme-primary)); font-weight: 500; }
.dashboard-panel-footer { margin-block-start: auto; padding: 12px 20px; border-block-start: 1px solid rgba(var(--v-theme-on-surface), .08); }
.dashboard-panel-footer > span { color: rgba(var(--v-theme-on-surface), .6); }
.dashboard-notifications { margin-block-start: 12px; }
.dashboard-notification-counts strong { display: block; font-size: 1.5rem; font-weight: 500; line-height: 1.4; }
.dashboard-notification-counts span { display: block; font-size: .8125rem; color: rgba(var(--v-theme-on-surface), .6); margin-block-start: 5px; }
.dashboard-channel { display: flex; align-items: center; gap: 12px; min-inline-size: 0; }
.dashboard-channel-copy { flex: 1 1 auto; min-inline-size: 0; }
.dashboard-channel > .v-avatar { flex: 0 0 auto; }
.dashboard-channel-copy strong, .dashboard-channel-copy > span { display: block; overflow-wrap: anywhere; }
.dashboard-channel-copy strong { font-size: .875rem; font-weight: 500; }
.dashboard-channel-copy > span { font-size: .75rem; color: rgba(var(--v-theme-on-surface), .6); margin-block-start: 4px; }
.dashboard-muted { color: rgba(var(--v-theme-on-surface), .6); font-size: .875rem; line-height: 1.8; }
.dashboard-footer { display: flex; justify-content: space-between; flex-wrap: wrap; gap: 10px; margin-block-start: 24px; color: rgba(var(--v-theme-on-surface), .6); font-size: .75rem; }
.dashboard-footer > span:first-child { display: flex; align-items: center; gap: 7px; }
.interactive-card { cursor: pointer; }
.interactive-card:focus-visible { outline: 3px solid rgba(var(--v-theme-primary), .32); outline-offset: 3px; }

.mp-toolbar { display: flex; align-items: center; justify-content: space-between; gap: 15px; inline-size: 100%; max-inline-size: none; }
.mp-toolbar-title, .mp-toolbar-actions { display: flex; align-items: center; gap: 9px; }
.mp-toolbar-title { font-size: .875rem; color: rgba(var(--v-theme-on-surface), .6); }
.mp-toolbar-actions { gap: 8px; }
.mp-nav-brand { display: flex; align-items: center; gap: 12px; padding: 24px 18px 28px; }
.mp-nav-brand strong { display: block; font-size: 1rem; font-weight: 600; }
.mp-nav-brand small { display: block; font-size: .75rem; color: rgba(var(--v-theme-on-surface), .6); margin-block-start: 4px; }
.mp-navigation-list { padding: 8px 12px; }
.mp-navigation-list :deep(.v-list-item__prepend) { margin-inline-end: 13px; }
.mp-nav-footer { text-align: center; font-size: .6875rem; color: rgba(var(--v-theme-on-surface), .6); padding: 18px; }
.profile-menu { inline-size:max-content; min-inline-size:0; padding:4px; border:1px solid rgba(var(--v-theme-on-surface),.08); border-radius:18px!important; box-shadow:0 8px 30px rgba(0,0,0,.12)!important; }
.profile-menu :deep(.v-list-item) { min-block-size: 38px; padding-inline: 10px; }
.profile-menu :deep(.v-list-item__prepend) { margin-inline-end: 0!important; }
.profile-menu :deep(.v-list-item__spacer) { inline-size:0!important; width:0!important; }
.profile-menu :deep(.v-list-item__prepend > .v-icon) { margin-inline-end:8px; }
.profile-menu :deep(.v-list-item-title) { font-size: .875rem; line-height: 1.5; }
.auth-page { position: relative; isolation: isolate; display: flex; flex-direction: column; align-items: center; justify-content: center; min-block-size: 100dvh; padding-block: max(24px, env(safe-area-inset-top)) max(24px, env(safe-area-inset-bottom)); background-color: #293242; background-image: url("/appearance/wallpaper/image"); background-size: cover; background-position: center; }
.auth-page::before { content: ""; position: absolute; z-index: -1; inset: 0; background: rgba(0, 0, 0, .14); }
.auth-theme { color: white; background: rgba(0, 0, 0, .12); position: absolute; inset-block-start: calc(env(safe-area-inset-top, 0px) + 16px); inset-inline-end: 16px; z-index: 2; }
.auth-wrapper { inline-size: 100%; padding-inline: 16px; display: flex; align-items: center; justify-content: center; }
.auth-card { inline-size: 100%; padding: 2.25rem; border: 1px solid rgba(255, 255, 255, .24); border-radius: 20px; background: rgba(var(--v-theme-surface), .86)!important; backdrop-filter: blur(22px) saturate(115%); -webkit-backdrop-filter: blur(22px) saturate(115%); box-shadow: 0 20px 54px rgba(0, 0, 0, .24); }
.auth-head { display: flex; flex-direction: column; align-items: center; gap: 12px; text-align: center; margin-block-end: 24px; }
.auth-head h1 { font-size: 1.75rem; font-weight: 500; margin: 0; line-height: 1.4; }
.auth-head p { font-size: .875rem; color: rgba(var(--v-theme-on-surface), .6); margin: 0; }
.auth-form { min-inline-size: 0; }
.auth-input-field { position: relative; display: flex; overflow: hidden; align-items: center; min-block-size: 56px; border: 1px solid rgba(var(--v-theme-on-surface), .22); border-radius: 12px; background: rgba(var(--v-theme-surface), .8); margin-block-end: 20px; }
.auth-input-field:focus-within { border-color: rgb(var(--v-theme-primary)); box-shadow: inset 0 0 0 1px rgb(var(--v-theme-primary)); }
.auth-input-icon { position: absolute; z-index: 1; inset-block-start: 16px; inset-inline-start: 16px; color: rgba(var(--v-theme-on-surface), .6); pointer-events: none; }
.auth-input-field input { display: block; border: 0; appearance: none; background: transparent; block-size: 54px; inline-size: 100%; min-inline-size: 0; color: rgb(var(--v-theme-on-surface)); font: inherit; font-size: .9375rem; line-height: 1.5; outline: none; padding-block: 0; padding-inline: 48px 16px; }
.auth-input-field--password input { padding-inline-end: 48px; }
.auth-input-field input::placeholder { color: rgba(var(--v-theme-on-surface), .6); opacity: 1; }
.auth-input-field input:-webkit-autofill { -webkit-text-fill-color: rgb(var(--v-theme-on-surface)); caret-color: rgb(var(--v-theme-on-surface)); transition: background-color 9999s ease-in-out 0s; }
.auth-password-toggle { position: absolute; z-index: 2; display: inline-flex; align-items: center; justify-content: center; border: 0; border-radius: 50%; background: transparent; block-size: 40px; inline-size: 40px; color: rgba(var(--v-theme-on-surface), .6); inset-block-start: 8px; inset-inline-end: 8px; padding: 0; cursor: pointer; }
.auth-password-toggle:hover, .auth-password-toggle:focus-visible { color: rgb(var(--v-theme-primary)); background: rgba(var(--v-theme-on-surface), .08); outline: none; }
.auth-remember { display: inline-flex; align-items: center; gap: 8px; margin-block: 0 20px; color: rgba(var(--v-theme-on-surface), .75); font-size: .875rem; cursor: pointer; }
.auth-remember input { inline-size: 16px; block-size: 16px; accent-color: rgb(var(--v-theme-primary)); }
.auth-submit { font-weight: 600; box-shadow: 0 6px 18px rgba(var(--v-theme-primary), .2); }
.auth-note { margin-block: 18px 0; font-size: .75rem; color: rgba(var(--v-theme-on-surface), .54); line-height: 1.7; text-align: center; }

@media (width < 960px) {
  .dashboard-dialog-card--fullscreen .dashboard-dialog-header { padding-block-start: max(12px, env(safe-area-inset-top)); }
  .dashboard-dialog-footer { padding-block-end: max(14px, calc(env(safe-area-inset-bottom) + 12px)); }
}
@media (width < 600px) {
  .dashboard-header { align-items: flex-start; }
  .dashboard-header > .v-chip { display: none; }
  .dashboard-section-heading { gap: 12px; align-items: flex-start; }
  .dashboard-actions > span { display: none; }
  .dashboard-stat-heading { font-size: .8125rem; gap: 8px; }
  .dashboard-stat-value { font-size: 1.5rem; }
  .dashboard-dialog-body { padding: 16px; }
  .dashboard-dialog-body--logs { padding: 0; }
  .dashboard-dialog-header :deep(.v-card-item__prepend) { padding-inline-end: 8px; }
  .dashboard-dialog-header :deep(.v-card-title) { font-size: 1.125rem; }
  .dashboard-dialog-footer { padding-inline: 16px; }
  .dashboard-save-state { flex-basis: 100%; font-size: .75rem; }
  .dashboard-address-row { gap: 8px; grid-template-columns: 40px minmax(0, 1fr); }
  .dashboard-address-list > div { gap: 8px; grid-template-columns: 70px minmax(0, 1fr); }
  .dashboard-panel-footer { flex-wrap: wrap; gap: 6px; }
  .mp-toolbar-title { gap: 3px; font-size: .8125rem; }
  .mp-toolbar-actions { gap: 2px; }
}
@media(max-width:600px){.dashboard-network-addresses .dashboard-address-row{grid-template-columns:1fr;align-items:stretch;gap:2px}.dashboard-network-addresses .dashboard-address-value{inline-size:100%;max-inline-size:100%}}
@media (width <= 480px) { .auth-card { padding: 1.5rem; border-radius: 16px; } .auth-head h1 { font-size: 1.5rem; } .auth-wrapper { padding-inline: 12px; } }
@media (width <= 480px) and (height <= 600px) { .auth-card { padding-block: 16px; } .auth-head { margin-block-end: 16px; gap: 8px; } .auth-input-field { margin-block-end: 14px; } }
.dashboard-log-dialog-card { flex:none!important; block-size:min(88dvh,52rem)!important; min-block-size:0!important; max-block-size:calc(100dvh - 32px)!important; overflow:hidden!important; }
.dashboard-log-dialog-card.dashboard-dialog-card--fullscreen { block-size:100dvh!important; max-block-size:100dvh!important; }
.dashboard-log-dialog-header { min-block-size: 56px; padding-block: 8px; }
.dashboard-log-dialog-header :deep(.v-card-title) { font-size: 1.125rem; }
@supports not (backdrop-filter: blur(1px)) { .auth-card { background: rgb(var(--v-theme-surface))!important; } }
.overview-top { margin-block-end: 16px; }
.overview-top-column { display:flex; flex-direction:column; }
.overview-current :deep(.v-card-title), .overview-line-health :deep(.v-card-title), .overview-statistics :deep(.v-card-title), .overview-quick-card :deep(.v-card-title), .overview-service :deep(.v-card-title), .overview-main-card :deep(.v-card-title) { font-size:1rem; font-weight:500; line-height:1.5; }
.overview-current-body { padding-block-start:0; }
.overview-egress-name { font-size:1.75rem; line-height:1.5; font-weight:600; }
.overview-egress-name > span { margin-inline-start:10px; font-size:.9375rem; font-weight:400; color:rgba(var(--v-theme-on-surface),.6); }
.overview-current-caption { display:flex; align-items:center; gap:10px; color:rgba(var(--v-theme-on-surface),.6); font-size:.8125rem; margin-block-start:2px; }
.overview-caption { font-size:.75rem; line-height:1.7; color:rgba(var(--v-theme-on-surface),.6); margin-bottom:0; }
.overview-line-health { flex:1 1 auto; }
.overview-health-line { display:flex; align-items:center; justify-content:space-between; gap:10px; padding-block:12px; border-block-end:1px solid rgba(var(--v-theme-on-surface),.06); }
.overview-health-line:first-child { padding-block-start:0; }
.overview-health-line > div { min-inline-size:0; }
.overview-health-line strong { display:block; font-size:.875rem; font-weight:500; line-height:1.6; }
.overview-health-line code { display:block; color:rgba(var(--v-theme-on-surface),.6); font-size:.75rem; line-height:1.7; margin-block-start:3px; }
.overview-health-foot { display:flex; justify-content:space-between; flex-wrap:wrap; gap:8px; padding-block-start:14px; color:rgba(var(--v-theme-on-surface),.6); font-size:.75rem; }
.overview-statistic { display:flex; align-items:center; gap:10px; min-inline-size:0; }
.overview-statistic > div { min-inline-size:0; }
.overview-statistic > div > span { color:rgba(var(--v-theme-on-surface),.6); font-size:.75rem; }
.overview-statistic strong { display:block; font-size:1.375rem; font-weight:500; line-height:1.3; white-space:nowrap; }
.overview-statistic strong small { font-size:.8125rem; margin-inline-start:5px; color:rgba(var(--v-theme-on-surface),.6); }
.overview-statistic p { margin:3px 0 0; font-size:.6875rem; color:rgba(var(--v-theme-on-surface),.6); line-height:1.5; }
.overview-network .dashboard-address-value { font-size:.8125rem; }
.overview-quick-col { flex:0 0 12.5%; max-inline-size:12.5%; }
.overview-quick-button { min-inline-size:0; padding:3px 0; }
.overview-quick-content { display:flex; flex-direction:column; align-items:center; gap:7px; padding-block:2px; }
.overview-quick-content > span { font-size:.75rem; font-weight:400; color:rgba(var(--v-theme-on-surface),.6); }
.overview-service-field > span { display:block; font-size:.75rem; color:rgba(var(--v-theme-on-surface),.6); margin-block-end:3px; }
.overview-service-field strong { display:block; font-size:.8125rem; font-weight:500; line-height:1.6; overflow-wrap:anywhere; }
.overview-main-card { margin-block-end:20px; }
@media(max-width:600px) { .overview-quick-col { flex-basis:25%; max-inline-size:25%; margin-block-end:8px; }.overview-health-line{padding-block:10px}.overview-egress-name{font-size:1.5rem}.overview-network .dashboard-address-row{grid-template-columns:1fr;gap:2px}.overview-statistic{gap:8px} }
.dashboard-channel-avatar { flex:0 0 auto; background:transparent; }
.ikuai-resource-row { display:flex; flex-wrap:wrap; align-items:center; gap:16px; margin-block-start:12px; color:rgba(var(--v-theme-on-surface),.6); font-size:.75rem; }
.ikuai-resource-row > span { display:inline-flex; align-items:center; gap:5px; }
</style>
