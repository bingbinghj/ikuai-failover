import { computed, ref, watch, type Ref } from 'vue'
import type { Notice } from './model'

// Store event identifiers only; credentials and event bodies never enter browser storage.
export function useNotificationReadState(notices: Ref<Notice[]>, account: Ref<string>) {
  const read = ref(new Set<string>())
  function storageKey() { return `ikuai-failover-notifications-read:${account.value}` }
  watch(account, () => {
    try {
      const stored: unknown = JSON.parse(localStorage.getItem(storageKey()) || '[]')
      read.value = new Set(Array.isArray(stored) ? stored.filter((item): item is string => typeof item === 'string').slice(-400) : [])
    } catch { read.value = new Set() }
  }, { immediate: true })
  function persist() {
    try { localStorage.setItem(storageKey(), JSON.stringify([...read.value].slice(-400))) } catch { /* Read status works for this visit when storage is unavailable. */ }
  }
  function isRead(id: string) { return read.value.has(id) }
  function markRead(id: string) { read.value = new Set([...read.value, id].slice(-400)); persist() }
  function markAllRead() { read.value = new Set([...read.value, ...notices.value.map(notice => notice.id)].slice(-400)); persist() }
  const unread = computed(() => notices.value.filter(notice => !read.value.has(notice.id)).length)
  return { unread, isRead, markRead, markAllRead }
}
