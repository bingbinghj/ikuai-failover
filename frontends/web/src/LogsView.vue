<script setup lang="ts">
import { computed, nextTick, onMounted, onUnmounted, ref, watch } from 'vue'
import { useDisplay, useTheme } from 'vuetify'

const props = withDefaults(defineProps<{
  request: (path: string, init?: RequestInit) => Promise<unknown>
  mode?: 'realtime' | 'history'
  active?: boolean
  compact?: boolean
}>(), { mode: 'realtime', active: true, compact: false })
interface LogFile { name: string; size_bytes: number; modified_ms: number; active: boolean }
interface LogRecord { timestamp_ms: number; level: string; message: string; file?: string; key: string }
const levels = ['ALL', 'DEBUG', 'INFO', 'WARNING', 'ERROR']
const levelOptions = levels.map(value => ({ title: value === 'ALL' ? '全部级别' : value, value }))
const files = ref<LogFile[]>([])
const records = ref<LogRecord[]>([])
const selected = ref<string | null>('service.log')
const level = ref('ALL')
const keyword = ref('')
const lines = ref(600)
const paused = ref(false)
const follow = ref(true)
const pendingCount = ref(0)
const busy = ref(false)
const error = ref('')
const truncated = ref(false)
const logPanel = ref<HTMLElement | null>(null)
const realtime = computed(() => props.mode === 'realtime')
const latestFile = computed(() => [...files.value].sort((left, right) => Number(right.active) - Number(left.active) || Number(isCurrentName(right.name)) - Number(isCurrentName(left.name)) || right.modified_ms - left.modified_ms)[0])
const currentFile = computed(() => realtime.value ? latestFile.value?.name ?? 'service.log' : selected.value)
const hasFilter = computed(() => keyword.value.trim() !== '' || level.value !== 'ALL')
const visibleRecords = computed(() => realtime.value ? records.value.filter(matchesFilter) : records.value)
const fileOptions = computed(() => [
  { title: '全部日志文件', value: 'all' },
  ...[...files.value].sort((left, right) => Number(right.name === latestFile.value?.name) - Number(left.name === latestFile.value?.name) || right.modified_ms - left.modified_ms).map(file => ({ title: file.name, value: file.name })),
])
const lineOptions = [100, 300, 600, 1000].map(value => ({ title: value + ' 条', value }))
let alive = true
let selectionInitialized = false
let generation = 0
let queued = false
let resetPending = false
let applyingScroll = false
let timer: ReturnType<typeof setInterval> | undefined
let searchTimer: ReturnType<typeof setTimeout> | undefined

function object(value: unknown): Record<string, unknown> { return value !== null && typeof value === 'object' ? value as Record<string, unknown> : {} }
function validFile(name: unknown): name is string { return typeof name === 'string' && /^(?:service(?:\.[1-9])?\.log|server\.log(?:\.[1-9])?)$/.test(name) }
function isCurrentName(name: string) { return name === 'service.log' || name === 'server.log' }
function failure(reason: unknown) {
  const data = object(reason), payload = object(data.payload)
  return data.status === 401 ? '登录已过期，请重新登录。' : typeof payload.message === 'string' ? payload.message : '日志读取失败，请刷新重试。'
}
function date(value: number) { return value > 0 ? new Date(value).toLocaleString('zh-CN', { hour12: false }) : '暂无记录' }
function normalizeLevel(value: string) { return value === 'WARN' ? 'WARNING' : value }
function matchesFilter(record: LogRecord) {
  return (level.value === 'ALL' || record.level === level.value)
    && (!keyword.value.trim() || [record.message, record.level, record.file ?? '', date(record.timestamp_ms)].join(' ').toLocaleLowerCase().includes(keyword.value.trim().toLocaleLowerCase()))
}
function decodeRecords(value: unknown): LogRecord[] {
  if (!Array.isArray(value)) return []
  const occurrences = new Map<string, number>()
  return value.map(object).filter(record => typeof record.message === 'string').map(record => {
    const timestamp_ms = Number(record.timestamp_ms) || 0
    const level = normalizeLevel(String(record.level))
    const message = String(record.message)
    const file = validFile(record.file) ? record.file : undefined
    const identity = JSON.stringify([timestamp_ms, level, message, file ?? ''])
    const occurrence = occurrences.get(identity) ?? 0
    occurrences.set(identity, occurrence + 1)
    return { timestamp_ms, level, message, file, key: identity + ':' + occurrence }
  })
}
function scrollState() {
  const panel = logPanel.value
  if (!panel) return null
  const rows = Array.from(panel.querySelectorAll<HTMLElement>('[data-log-key]'))
  const row = rows.find(item => item.offsetTop + item.offsetHeight >= panel.scrollTop)
  return { top: panel.scrollTop, key: row?.dataset.logKey, offset: row ? row.offsetTop - panel.scrollTop : 0 }
}
async function updateRecords(next: LogRecord[], reset: boolean) {
  const previous = new Set(records.value.map(record => record.key))
  const position = follow.value ? null : scrollState()
  const fresh = reset ? 0 : next.filter(record => !previous.has(record.key) && (!realtime.value || matchesFilter(record))).length
  records.value = next
  if (reset) { follow.value = true; pendingCount.value = 0 }
  else if (!follow.value) pendingCount.value += fresh
  applyingScroll = true
  await nextTick()
  const panel = logPanel.value
  if (panel) {
    if (follow.value) panel.scrollTop = panel.scrollHeight
    else if (position) {
      const row = Array.from(panel.querySelectorAll<HTMLElement>('[data-log-key]')).find(item => item.dataset.logKey === position.key)
      panel.scrollTop = row ? row.offsetTop - position.offset : position.top
    }
  }
  requestAnimationFrame(() => { applyingScroll = false })
}
function onScroll() {
  const panel = logPanel.value
  if (!panel || applyingScroll) return
  follow.value = panel.scrollHeight - panel.scrollTop - panel.clientHeight <= 32
  if (follow.value) pendingCount.value = 0
}
function latest() {
  follow.value = true; pendingCount.value = 0
  if (logPanel.value) logPanel.value.scrollTop = logPanel.value.scrollHeight
}
async function refresh(reset = false) {
  if (!alive || !props.active) return
  if (reset) resetPending = true
  if (busy.value) { queued = true; return }
  const version = ++generation
  const resetView = resetPending
  resetPending = false
  busy.value = true
  try {
    const listing = object(await props.request('/logs/files'))
    if (!alive || version !== generation) return
    files.value = Array.isArray(listing.files) ? listing.files.map(object).filter(file => validFile(file.name)).map(file => ({
      name: String(file.name), size_bytes: Number(file.size_bytes) || 0,
      modified_ms: Number(file.modified_ms) || 0, active: file.active === true,
    })) : []
    if (!selectionInitialized || (selected.value !== 'all' && !files.value.some(file => file.name === selected.value))) {
      selected.value = latestFile.value?.name ?? null
      selectionInitialized = true
    }
    if (!files.value.length) { records.value = []; truncated.value = false; error.value = ''; return }
    if (!currentFile.value) { records.value = []; truncated.value = false; error.value = ''; return }
    const query = new URLSearchParams({ file: currentFile.value, lines: String(lines.value), level: realtime.value ? 'ALL' : level.value })
    if (!realtime.value && keyword.value.trim()) query.set('search', keyword.value.trim())
    const result = object(await props.request('/logs/tail?' + query))
    if (!alive || version !== generation) return
    await updateRecords(decodeRecords(result.lines), resetView)
    truncated.value = result.truncated === true
    error.value = ''
  } catch (reason: unknown) { if (alive && version === generation) error.value = failure(reason) }
  finally {
    if (alive) {
      busy.value = false
      if (queued) { queued = false; void refresh() }
    }
  }
}
function changeFilter() { generation += 1; resetPending = true; queued = busy.value; if (!busy.value) void refresh(true) }
watch([selected, lines], changeFilter)
watch(level, () => {
  if (!realtime.value) changeFilter()
  else { pendingCount.value = 0; if (follow.value) void nextTick(latest) }
})
watch(keyword, () => {
  if (realtime.value) { pendingCount.value = 0; if (follow.value) void nextTick(latest); return }
  generation += 1
  if (searchTimer) clearTimeout(searchTimer)
  searchTimer = setTimeout(changeFilter, 300)
})
watch(paused, value => { if (!value && realtime.value) void refresh() })
watch(() => props.active, value => { if (value && (!paused.value || !realtime.value)) void refresh() })
onMounted(() => { void refresh(true); timer = setInterval(() => { if (realtime.value && props.active && !paused.value && !document.hidden) void refresh() }, 3000) })
onUnmounted(() => { alive = false; generation += 1; if (timer) clearInterval(timer); if (searchTimer) clearTimeout(searchTimer) })

const display = useDisplay()
const theme = useTheme()
const isDarkTheme = computed(() => theme.global.current.value.dark)
const logColorMap: Record<string, string> = { DEBUG: 'secondary', INFO: 'success', WARNING: 'warning', ERROR: 'error' }
const groupedLogs = computed(() => {
  const groups: { key: string; level: string; second: string; timestamp: number; items: LogRecord[] }[] = []
  for (const record of visibleRecords.value) {
    const second = record.timestamp_ms ? date(record.timestamp_ms) : '…'
    const previous = groups.at(-1)
    if (previous && previous.second === second && previous.level === record.level && record.timestamp_ms - previous.timestamp <= 100) {
      previous.items.push(record)
      previous.timestamp = record.timestamp_ms
    } else groups.push({ key: record.key, level: record.level, second, timestamp: record.timestamp_ms, items: [record] })
  }
  return groups
})
function inlineTime(record: LogRecord) {
  if (!record.timestamp_ms) return ''
  return new Date(record.timestamp_ms).toLocaleTimeString('zh-CN', { hour12: false }) + '.' + String(record.timestamp_ms % 1000).padStart(3, '0')
}
function sourceLabel(record: LogRecord) { return record.message.match(/^\[([^\]]+)\]\s*/)?.[1] ?? '' }
function messageBody(record: LogRecord) { return record.message.replace(/^\[[^\]]+\]\s*/, '') }
const emptyText = computed(() => !realtime.value && selected.value === null
  ? '暂无日志记录'
  : hasFilter.value ? '没有匹配的日志' : realtime.value ? '等待实时日志' : '所选文件暂无日志记录')

</script>

<template>
  <section class="mp-logging-content" :class="{ compact }">
    <VProgressLinear v-if="realtime && !paused && active" class="logging-live-progress" indeterminate color="primary" height="1" />
    <div class="logging-view" :class="{ 'is-dark-theme': isDarkTheme }">
      <div v-if="!realtime" class="logging-history-toolbar px-3">
        <VSelect v-model="selected" class="logging-file-select" :items="fileOptions" item-title="title" item-value="value" label="日志文件" placeholder="暂无日志文件" persistent-placeholder prepend-inner-icon="mdi-file-document-outline" variant="plain" density="compact" hide-details :disabled="!files.length" />
        <VSelect v-model="lines" class="logging-count-select" :items="lineOptions" item-title="title" item-value="value" label="显示条数" variant="plain" density="compact" hide-details />
      </div>
      <div class="logging-toolbar px-3">
        <VBtnToggle v-if="display.mdAndUp.value" v-model="level" mandatory divided density="compact" variant="text" selected-class="logging-level-toggle__button--active" class="logging-level-toggle">
          <VBtn v-for="item in levels" :key="item" :value="item" :class="'logging-level-toggle__button--' + item.toLowerCase()" class="logging-level-toggle__button" color="default">{{ item === 'ALL' ? '全部' : item }}</VBtn>
        </VBtnToggle>
        <VSelect v-else v-model="level" :items="levelOptions" item-title="title" item-value="value" aria-label="日志级别" density="compact" variant="plain" hide-details class="logging-level-select" :class="level === 'ALL' ? 'text-primary' : 'text-' + (logColorMap[level] || 'secondary')" :menu-props="{ width: 'auto' }" />
        <VTextField :model-value="keyword" class="logging-search" density="compact" variant="plain" hide-details clearable prepend-inner-icon="mdi-magnify" placeholder="搜索日志" aria-label="搜索日志" maxlength="64" @update:model-value="keyword = String($event ?? '')" />
        <span class="logging-count" :title="truncated ? '显示最近的日志记录' : '当前显示的日志记录'">{{ visibleRecords.length }} 条</span>
        <div class="logging-toolbar-actions">
          <VBtn v-if="realtime" variant="text" color="default" icon class="logging-stream-action" :class="{ 'is-live': !paused }" :aria-label="paused ? '恢复实时日志' : '暂停实时日志'" :title="paused ? '恢复实时日志' : '暂停实时日志'" @click="paused = !paused"><VIcon :icon="paused ? 'mdi-play' : 'mdi-pause'" /></VBtn>
          <VBtn variant="text" color="default" icon class="logging-stream-action" :loading="busy" aria-label="刷新日志" title="刷新日志" @click="refresh()"><VIcon icon="mdi-refresh" /></VBtn>
        </div>
      </div>
      <VAlert v-if="error" type="error" variant="tonal" density="compact" class="logging-error mx-3 mb-2">{{ error }}</VAlert>
      <div ref="logPanel" class="logging-shell is-wrap" role="log" :aria-label="realtime ? '实时日志' : '历史日志'" :aria-busy="busy" :aria-live="realtime && !paused ? 'polite' : 'off'" @scroll.passive="onScroll">
        <div v-if="busy && !records.length" class="logging-empty"><VProgressCircular indeterminate size="20" width="2" /><span>正在读取日志…</span></div>
        <div v-else-if="!groupedLogs.length" class="logging-empty"><VIcon :icon="hasFilter ? 'mdi-filter-remove-outline' : 'mdi-console-line'" size="20" /><span>{{ emptyText }}</span></div>
        <div v-else class="logging-list">
          <div v-for="(group, index) in groupedLogs" :key="group.key" class="logging-record">
            <div class="logging-record-time">{{ group.second }}</div>
            <div class="logging-record-panel" :class="index % 2 === 0 ? 'is-even' : 'is-odd'">
              <div class="logging-record-accent" :class="['level-' + group.level.toLowerCase(), { 'is-burst': group.items.length > 1 }]" />
              <div class="logging-record-lines">
                <div v-for="record in group.items" :key="record.key" :data-log-key="record.key" class="logging-record-line">
                  <div class="logging-record-level" :class="'level-' + record.level.toLowerCase()">{{ record.level }}:</div>
                  <div class="logging-record-body"><span class="logging-record-inline-time">{{ inlineTime(record) }}</span><span v-if="currentFile === 'all' && record.file" class="logging-record-file"> [{{ record.file }}]</span><span v-if="sourceLabel(record)" class="logging-record-source"> {{ sourceLabel(record) }} -</span> {{ messageBody(record) }}</div>
                </div>
              </div>
            </div>
          </div>
        </div>
        <div v-if="realtime && !follow && visibleRecords.length" class="logging-latest-action"><VBtn size="small" color="primary" variant="elevated" prepend-icon="mdi-arrow-down" @click="latest">查看最新{{ pendingCount ? '（' + pendingCount + '）' : '' }}</VBtn></div>
      </div>
    </div>
  </section>
</template>

<style scoped>

.mp-logging-content { display:flex; flex-direction:column; flex:1 1 0%; block-size:0; min-block-size:0; overflow:hidden; }
.logging-view {
  --logging-shell-bg:rgba(var(--v-theme-surface),0.96);
  --logging-record-bg-even:rgba(var(--v-theme-surface-variant),0.01);
  --logging-record-bg-odd:rgba(var(--v-theme-surface-variant),0.005);
  --logging-text:rgba(var(--v-theme-on-surface),0.88);
  --logging-muted:rgba(var(--v-theme-on-surface),0.56);
  --logging-border:rgba(var(--v-theme-on-surface),0.1);
  display:flex; flex-direction:column; flex:1 1 0%; block-size:0; min-block-size:0;
}
.logging-view.is-dark-theme {
  --logging-shell-bg:rgba(var(--v-theme-surface),0.72);
  --logging-record-bg-even:rgba(var(--v-theme-on-surface),0.02);
  --logging-record-bg-odd:rgba(var(--v-theme-on-surface),0.008);
  --logging-border:rgba(var(--v-theme-on-surface),0.12);
}
.logging-toolbar { display:flex; flex-wrap:wrap; align-items:center; gap:0.5rem; flex:none; padding-block:0.5rem; }
.logging-search { flex:1 1 9rem; min-inline-size:5rem; }
.logging-search :deep(.v-field), .logging-level-select :deep(.v-field) { border-radius:var(--app-field-radius); background:transparent!important; box-shadow:none!important; }
.logging-search :deep(.v-field__outline), .logging-search :deep(.v-field__overlay), .logging-level-select :deep(.v-field__outline), .logging-level-select :deep(.v-field__overlay) { display:none; }
.logging-search :deep(.v-field__input), .logging-level-select :deep(.v-field__input) { padding-inline-start:0; }
.logging-level-select { flex:0 0 7rem; min-inline-size:7rem; }
.logging-level-toggle { flex:0 0 auto; overflow:hidden; border:1px solid rgba(var(--v-theme-on-surface),0.14); border-radius:var(--app-control-radius); background:transparent; block-size:2rem; box-shadow:none!important; }
.logging-level-toggle :deep(.logging-level-toggle__button) { border-color:rgba(var(--v-theme-on-surface),0.1); border-radius:0; block-size:100%; color:var(--logging-muted); font-size:12px; font-weight:600; letter-spacing:0; min-inline-size:auto; padding-inline:1.125rem; text-transform:none; }
.logging-level-toggle :deep(.logging-level-toggle__button--debug) { color:rgb(var(--v-theme-secondary)); }
.logging-level-toggle :deep(.logging-level-toggle__button--info) { color:rgb(var(--v-theme-success)); }
.logging-level-toggle :deep(.logging-level-toggle__button--warning) { color:rgb(var(--v-theme-warning)); }
.logging-level-toggle :deep(.logging-level-toggle__button--error) { color:rgb(var(--v-theme-error)); }
.logging-level-toggle :deep(.logging-level-toggle__button--active) { background:rgba(var(--v-theme-primary),0.16); color:rgb(var(--v-theme-primary)); }
.logging-toolbar-actions { display:flex; align-items:center; gap:0.125rem; flex:none; }
.logging-stream-action { color:var(--logging-muted); inline-size:2rem; block-size:2rem; }
.logging-stream-action.is-live { color:rgb(var(--v-theme-success)); }
.logging-count { flex:none; font-size:12px; line-height:1.5rem; color:var(--logging-muted); white-space:nowrap; }
.logging-history-toolbar { display:flex; align-items:center; flex:none; gap:1rem; padding-block:0.75rem 0.5rem; }
.logging-file-select { flex:1 1 auto; min-inline-size:0; }
.logging-count-select { flex:0 0 8rem; }
.logging-history-toolbar :deep(.v-field__input) { font-size:13px; }
.logging-error { flex:none; }
.logging-shell { position:relative; overflow:auto; flex:1 1 auto; min-block-size:0; padding:0.875rem; border:1px solid var(--logging-border); background:linear-gradient(180deg,var(--logging-shell-bg),rgba(var(--v-theme-surface),0.9)); overscroll-behavior:contain; scrollbar-gutter:stable; }
.logging-empty { display:flex; align-items:center; justify-content:center; block-size:100%; color:var(--logging-muted); gap:0.5rem; }
.logging-list { display:flex; flex-direction:column; gap:0.375rem; min-inline-size:100%; }
.logging-record { display:grid; align-items:start; grid-template-columns:11rem minmax(0,1fr); }
.logging-record-time { color:rgb(var(--v-theme-primary)); font-family:'JetBrains Mono','Fira Code',Consolas,monospace; font-size:13px; font-weight:600; line-height:1.5; padding-block:0.625rem; white-space:nowrap; }
.logging-record-panel { display:flex; align-items:stretch; border:1px solid rgba(var(--v-theme-on-surface),0.04); border-radius:var(--app-surface-radius); gap:0.75rem; min-inline-size:0; padding:0.5rem; }
.logging-record-panel.is-even { background:var(--logging-record-bg-even); }
.logging-record-panel.is-odd { background:var(--logging-record-bg-odd); }
.is-dark-theme .logging-record-panel { border-color:rgba(var(--v-theme-on-surface),0.08); }
.logging-record-accent { flex:0 0 auto; align-self:flex-start; border-radius:999px; background-color:rgba(var(--v-theme-on-surface),0.24); block-size:0.5rem; inline-size:0.5rem; margin-block-start:0.45rem; }
.logging-record-accent.is-burst { align-self:stretch; block-size:auto; margin-block-start:0; }
.logging-record-lines { display:flex; flex:1 1 auto; flex-direction:column; gap:0.125rem; min-inline-size:0; }
.logging-record-line { display:flex; align-items:flex-start; color:var(--logging-text); font-family:'JetBrains Mono','Fira Code',Consolas,monospace; font-size:13px; gap:0.75rem; line-height:1.6; min-inline-size:0; }
.logging-record-level { flex:0 0 4rem; font-weight:700; min-inline-size:4rem; }
.logging-record-body { flex:1 1 auto; color:var(--logging-text); min-inline-size:0; overflow-wrap:anywhere; white-space:pre-wrap; }
.logging-record-inline-time, .logging-record-source, .logging-record-file { color:var(--logging-muted); }
.logging-record-level.level-debug { color:rgb(var(--v-theme-secondary)); }
.logging-record-level.level-info { color:rgb(var(--v-theme-success)); }
.logging-record-level.level-warning { color:rgb(var(--v-theme-warning)); }
.logging-record-level.level-error { color:rgb(var(--v-theme-error)); }
.logging-record-accent.level-debug { background-color:rgb(var(--v-theme-secondary)); }
.logging-record-accent.level-info { background-color:rgb(var(--v-theme-success)); }
.logging-record-accent.level-warning { background-color:rgb(var(--v-theme-warning)); }
.logging-record-accent.level-error { background-color:rgb(var(--v-theme-error)); }
.logging-latest-action { position:sticky; display:flex; justify-content:flex-end; inset-block-end:0.75rem; margin-block-start:0.75rem; pointer-events:none; }
.logging-latest-action :deep(.v-btn) { box-shadow:0 8px 24px rgba(15,23,42,0.16); pointer-events:auto; }
@media (width >= 960px) {
  .logging-search :deep(.v-field) { display:flex; align-items:center; block-size:2rem; min-block-size:2rem; }
  .logging-search :deep(.v-field__field), .logging-search :deep(.v-field__input), .logging-search :deep(.v-field__prepend-inner), .logging-search :deep(.v-field__clearable) { align-items:center; min-block-size:2rem; }
  .logging-search :deep(.v-field__input), .logging-search :deep(.v-field__prepend-inner), .logging-search :deep(.v-field__clearable) { padding-block:0; }
}
@media (width <= 960px) {
  .logging-record { gap:0.375rem; grid-template-columns:9.5rem minmax(0,1fr); }
  .logging-record-level { flex-basis:4.75rem; min-inline-size:4.75rem; }
}
@media (width <= 640px) {
  .logging-level-select { order:0; }
  .logging-count { order:1; margin-inline-start:auto; }
  .logging-toolbar-actions { order:1; }
  .logging-search { order:2; flex:1 1 100%; min-inline-size:0; inline-size:100%; }
  .logging-search :deep(.v-field__input) { min-inline-size:0; }
  .logging-history-toolbar { gap:0.75rem; }
  .logging-count-select { flex-basis:6rem; }
  .logging-record { gap:0.25rem; grid-template-columns:minmax(0,1fr); }
  .logging-record-time { padding-block:0; }
  .logging-record-panel { padding-inline:0.625rem; }
  .logging-record-line { gap:0.5rem; }
  .logging-shell { padding:0.625rem; }
}

</style>
