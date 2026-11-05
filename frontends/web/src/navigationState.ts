import { onMounted, onUnmounted, ref, watch } from 'vue'

export type NavigationSection = 'overview' | 'settings' | 'logs'
export type SettingsGroup = 'connection' | 'wan' | 'ipv6' | 'notifications' | 'management'
export type LogTab = 'realtime' | 'history' | 'notifications'
interface NavigationPreference { section: NavigationSection; settingsGroup: SettingsGroup; logView: LogTab }
const key = 'ikuai-failover-navigation'
const groups: SettingsGroup[] = ['connection', 'wan', 'ipv6', 'notifications', 'management']
const tabs: LogTab[] = ['realtime', 'history', 'notifications']
function stored(): NavigationPreference {
  const fallback: NavigationPreference = { section: 'overview', settingsGroup: 'connection', logView: 'realtime' }
  try {
    const data = JSON.parse(localStorage.getItem(key) ?? '{}') as Partial<NavigationPreference>
    return { section: ['overview', 'settings', 'logs'].includes(String(data.section)) ? data.section! : fallback.section, settingsGroup: groups.includes(data.settingsGroup!) ? data.settingsGroup! : fallback.settingsGroup, logView: tabs.includes(data.logView!) ? data.logView! : fallback.logView }
  } catch { return fallback }
}
function parse(hash: string, preference: NavigationPreference): NavigationPreference | null {
  if (hash === '#/overview') return { ...preference, section: 'overview' }
  const setting = hash.match(/^#\/settings\/(connection|wan|ipv6|notifications|management)$/)
  if (setting) return { ...preference, section: 'settings', settingsGroup: setting[1] as SettingsGroup }
  const log = hash.match(/^#\/logs\/(realtime|history|notifications)$/)
  if (log) return { ...preference, section: 'logs', logView: log[1] as LogTab }
  if (hash === '#/settings') return { ...preference, section: 'settings' }
  if (hash === '#/logs') return { ...preference, section: 'logs' }
  return null
}
function hashFor(state: NavigationPreference) { return state.section === 'settings' ? '#/settings/' + state.settingsGroup : state.section === 'logs' ? '#/logs/' + state.logView : '#/overview' }

export function useNavigationState() {
  const preference = stored()
  const initial = parse(window.location.hash, preference) ?? preference
  const section = ref<NavigationSection>(initial.section)
  const settingsGroup = ref<SettingsGroup>(initial.settingsGroup)
  const logView = ref<LogTab>(initial.logView)
  let receiving = false
  const current = (): NavigationPreference => ({ section: section.value, settingsGroup: settingsGroup.value, logView: logView.value })
  function persist() { try { localStorage.setItem(key, JSON.stringify(current())) } catch { /* Private browser modes may disallow persistence. */ } }
  function commit(replace = false) {
    persist()
    const hash = hashFor(current())
    if (window.location.hash !== hash) {
      if (replace) window.history.replaceState(window.history.state, '', window.location.pathname + window.location.search + hash)
      else window.location.hash = hash
    }
  }
  function receive() {
    const state = parse(window.location.hash, current())
    if (!state) { commit(true); return }
    receiving = true
    section.value = state.section; settingsGroup.value = state.settingsGroup; logView.value = state.logView
    receiving = false
    persist()
  }
  watch([section, settingsGroup, logView], () => { if (!receiving) commit() }, { flush: 'sync' })
  function navigate(target: NavigationSection) { section.value = target; commit() }
  function selectSettings(value: string) { if (groups.includes(value as SettingsGroup)) settingsGroup.value = value as SettingsGroup }
  onMounted(() => { commit(true); window.addEventListener('hashchange', receive); window.addEventListener('popstate', receive) })
  onUnmounted(() => { window.removeEventListener('hashchange', receive); window.removeEventListener('popstate', receive) })
  return { section, settingsGroup, logView, navigate, selectSettings }
}
