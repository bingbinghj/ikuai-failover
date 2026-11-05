<script setup lang="ts">
import { computed, ref } from 'vue'
import { errorLabels, eventLabel, providerLabel, timeLabel, type Notice } from './model'

const props = defineProps<{
  notices: Notice[]
  loading: boolean
  error?: string
  unread: number
  isRead: (id: string) => boolean
  compact?: boolean
}>()
const emit = defineEmits<{ read: [id: string]; readAll: []; refresh: []; logs: [] }>()
const query = ref('')
const filter = ref<'all' | 'unread' | 'failed'>('all')
const expanded = ref(new Set<string>())
const limit = ref(20)
const failed = computed(() => props.notices.filter(notice => notice.status === 'failed' || notice.status === 'dropped' || notice.deliveries.some(delivery => !delivery.success)).length)
const filtered = computed(() => props.notices.filter(notice => {
  const text = [notice.title, notice.text, eventLabel(notice.kind), ...notice.deliveries.map(delivery => `${providerLabel(delivery.provider)} ${delivery.tag}`)].join(' ').toLocaleLowerCase()
  return (!query.value.trim() || text.includes(query.value.trim().toLocaleLowerCase()))
    && (filter.value !== 'unread' || !props.isRead(notice.id))
    && (filter.value !== 'failed' || notice.status === 'failed' || notice.status === 'dropped' || notice.deliveries.some(delivery => !delivery.success))
}))
const visible = computed(() => filtered.value.slice(0, limit.value))
function state(notice: Notice) {
  if (notice.pending.length) return { label: '等待发送', color: 'info', icon: 'clock' }
  if (notice.status === 'failed' || notice.status === 'dropped' || notice.deliveries.some(delivery => !delivery.success)) return { label: notice.status === 'dropped' ? '等待队列已满' : '发送失败', color: 'error', icon: 'alert' }
  if (notice.status === 'delivered' || notice.deliveries.some(delivery => delivery.success)) return { label: '发送完成', color: 'success', icon: 'check' }
  return { label: notice.status === 'filtered' ? '未推送' : '已记录', color: 'secondary', icon: 'bell' }
}
function toggle(notice: Notice) {
  const next = new Set(expanded.value)
  if (next.has(notice.id)) next.delete(notice.id)
  else { next.add(notice.id); emit('read', notice.id) }
  expanded.value = next
}

function statusIcon(notice: Notice) { return ({ clock: 'mdi-clock-outline', alert: 'mdi-alert-circle-outline', check: 'mdi-check-circle-outline', bell: 'mdi-bell-outline' } as Record<string, string>)[state(notice).icon] }
</script>

<template>
  <section class="mp-notification-view" :class="{ compact }">
    <VProgressLinear v-if="loading" indeterminate color="primary" height="1" />
    <div class="mp-notification-toolbar px-3">
      <VBtnToggle v-model="filter" mandatory divided density="compact" variant="text" selected-class="mp-notification-filter--active" class="mp-notification-filters" @update:model-value="limit = 20">
        <VBtn value="all" color="default">全部 <span class="ml-1">{{ notices.length }}</span></VBtn>
        <VBtn value="unread" color="default">未读 <span class="ml-1">{{ unread }}</span></VBtn>
        <VBtn value="failed" color="default">失败 <span class="ml-1">{{ failed }}</span></VBtn>
      </VBtnToggle>
      <VTextField :model-value="query" class="mp-notification-search" variant="plain" density="compact" hide-details clearable prepend-inner-icon="mdi-magnify" aria-label="搜索通知" placeholder="搜索通知内容或渠道" @update:model-value="query = String($event ?? ''); limit = 20" />
      <div class="mp-notification-toolbar-actions">
        <VBtn icon="mdi-refresh" color="default" variant="text" :loading="loading" aria-label="刷新通知" title="刷新通知" @click="emit('refresh')" />
        <VBtn icon="mdi-check-all" color="default" variant="text" :disabled="!unread" aria-label="全部标为已读" title="全部标为已读" @click="emit('readAll')" />
      </div>
    </div>
    <VAlert v-if="error" type="error" variant="tonal" density="compact" class="mx-3 mb-2">{{ error }}</VAlert>
    <div class="mp-notification-viewport">
      <div v-if="!visible.length" class="mp-notification-empty"><div class="mp-notification-empty__icon"><VIcon icon="mdi-bell-sleep-outline" size="22" /></div><div>{{ loading && !notices.length ? '正在读取通知…' : filter === 'unread' && !unread ? '通知已全部阅读' : notices.length ? '没有匹配的通知' : '暂无通知记录' }}</div></div>
      <div v-else class="mp-notification-list">
        <div v-for="notice in visible" :key="notice.id" class="mp-notification-virtual-item">
          <div class="mp-notification-row" :class="{ 'mp-notification-row--unread': !isRead(notice.id) }" role="button" tabindex="0" :aria-expanded="expanded.has(notice.id)" @click="toggle(notice)" @keydown.enter.prevent="toggle(notice)" @keydown.space.prevent="toggle(notice)">
            <div class="mp-notification-icon" :class="'text-' + state(notice).color"><VIcon :icon="statusIcon(notice)" size="22" /></div>
            <div class="mp-notification-content">
              <div class="mp-notification-title-row"><span class="mp-notification-title">{{ notice.title }}</span><span v-if="!isRead(notice.id)" class="mp-notification-unread-dot" /></div>
              <div class="mp-notification-text" :class="{ 'mp-notification-text--expanded': expanded.has(notice.id) }">{{ notice.text }}</div>
              <div class="mp-notification-meta"><span class="mp-notification-type">{{ eventLabel(notice.kind) }}</span><span>{{ state(notice).label }}</span><time :datetime="new Date(notice.timestamp).toISOString()">{{ timeLabel(notice.timestamp) }}</time></div>
              <div v-if="expanded.has(notice.id)" class="mp-notification-deliveries"><VChip v-for="delivery in notice.deliveries" :key="delivery.tag" :color="delivery.success ? 'success' : 'error'" variant="tonal" size="small" :prepend-icon="delivery.success ? 'mdi-check' : 'mdi-alert-circle-outline'">{{ providerLabel(delivery.provider) || delivery.tag }}{{ !delivery.success ? ' · ' + (errorLabels[delivery.errorCode] || '发送失败') : '' }}</VChip><VChip v-for="tag in notice.pending" :key="tag" color="info" variant="tonal" size="small" prepend-icon="mdi-clock-outline">{{ tag }} · 等待发送</VChip></div>
            </div>
          </div>
        </div>
      </div>
      <div v-if="visible.length < filtered.length" class="d-flex justify-center pa-3"><VBtn color="primary" variant="text" @click="limit += 20">加载更多</VBtn></div>
    </div>
  </section>
</template>

<style scoped>

.mp-notification-view { display:flex; flex-direction:column; block-size:100%; min-block-size:0; }
.mp-notification-toolbar { display:flex; align-items:center; flex-wrap:wrap; gap:0.5rem; padding-block:0.625rem; flex:none; }
.mp-notification-filters { flex:none; block-size:2rem; border:1px solid rgba(var(--v-theme-on-surface),0.14); border-radius:var(--app-control-radius,0.375rem); }
.mp-notification-filters :deep(.v-btn) { min-inline-size:auto; block-size:100%; padding-inline:0.75rem; font-size:0.75rem; font-weight:600; letter-spacing:0; text-transform:none; }
.mp-notification-filters :deep(.mp-notification-filter--active) { background:rgba(var(--v-theme-primary),0.16); color:rgb(var(--v-theme-primary)); }
.mp-notification-search { flex:1 1 9rem; min-inline-size:0; }
.mp-notification-search :deep(.v-field) { background:transparent; }
.mp-notification-search :deep(.v-field__input) { padding-inline-start:0; }
.mp-notification-toolbar-actions { display:flex; flex:none; gap:0.125rem; }
.mp-notification-toolbar-actions .v-btn { inline-size:2rem; block-size:2rem; }
.mp-notification-viewport { flex:1 1 auto; min-block-size:0; overflow:auto; overscroll-behavior:contain; border-block:1px solid rgba(var(--v-theme-on-surface),0.1); }
.mp-notification-deliveries { display:flex; flex-wrap:wrap; gap:0.375rem; margin-block-start:0.75rem; }
@media (width <= 640px) {
  .mp-notification-filters { order:0; }
  .mp-notification-toolbar-actions { order:1; margin-inline-start:auto; }
  .mp-notification-search { order:2; flex:1 1 100%; inline-size:100%; }
}

.mp-notification-panel {
  overflow: hidden;
}

.mp-notification-actions {
  display: inline-flex;
  align-items: center;
  gap: 4px;
}

.mp-notification-clear-count {
  color: rgba(var(--v-theme-on-surface), var(--v-disabled-opacity));
  font-size: 0.75rem;
  line-height: 1;
  margin-inline-start: 16px;
}

.mp-notification-list-container {
  overflow: hidden;
  max-block-size: min(560px, 62vh);
}

.mp-notification-list-scroll {
  max-block-size: min(560px, 62vh);
  min-block-size: 160px;
}

.mp-notification-initial-loading {
  display: flex;
  min-block-size: 160px;
  align-items: center;
  justify-content: center;
  flex-direction: column;
  gap: 12px;
}

.mp-notification-virtual-item {
  padding-block: 4px;
  padding-inline: 8px;
}

.mp-notification-virtual-item--section {
  padding-block: 10px 2px;
}

.mp-notification-section-heading {
  display: flex;
  align-items: center;
  color: rgba(var(--v-theme-on-surface), 0.42);
  gap: 8px;
  letter-spacing: 0;
  padding-inline: 10px;
}

.mp-notification-section-heading__title {
  font-size: 0.6875rem;
  font-weight: 500;
  line-height: 1.2;
}

.mp-notification-section-heading__count {
  color: rgba(var(--v-theme-on-surface), 0.34);
  font-size: 0.625rem;
  line-height: 1;
}

.mp-notification-row {
  position: relative;
  display: flex;
  align-items: flex-start;
  padding: 10px;
  border: 0;
  border-radius: 8px;
  background: transparent;
  color: inherit;
  cursor: pointer;
  gap: 12px;
  inline-size: 100%;
  text-align: start;
  transition:
    background-color 0.2s ease,
    transform 0.2s ease;
}

.mp-notification-row:hover {
  background: rgba(var(--v-theme-primary), 0.08);
}

.mp-notification-row--unread {
  background: rgba(var(--v-theme-error), 0.07);
}

.mp-notification-row--media {
  min-block-size: 0;
}

.mp-notification-media {
  overflow: hidden;
  flex: 0 0 56px;
  border-radius: 6px;
  background: rgba(var(--v-theme-on-surface), 0.06);
  block-size: 84px;
}

.mp-notification-media__image {
  block-size: 100%;
  inline-size: 100%;
}

.mp-notification-icon {
  display: grid;
  place-items: center;
}

.mp-notification-icon {
  flex: 0 0 40px;
  border-radius: 8px;
  background: rgba(var(--v-theme-on-surface), 0.06);
  block-size: 40px;
}

.mp-notification-content {
  flex: 1;
  min-inline-size: 0;
}

.mp-notification-title-row {
  display: flex;
  align-items: flex-start;
  gap: 8px;
  min-block-size: 24px;
}

.mp-notification-title {
  display: -webkit-box;
  overflow: hidden;
  flex: 1 1 auto;
  -webkit-box-orient: vertical;
  font-size: 0.925rem;
  font-weight: 600;
  line-height: 1.35;
  -webkit-line-clamp: 2;
  text-overflow: ellipsis;
  white-space: normal;
}

.mp-notification-unread-dot {
  flex: 0 0 7px;
  border-radius: 999px;
  background: rgb(var(--v-theme-error));
  block-size: 7px;
  inline-size: 7px;
  margin-block-start: 0.45rem;
}

.mp-notification-text {
  display: block;
  overflow: hidden;
  padding: 0;
  border: 0;
  background: transparent;
  block-size: auto;
  color: rgba(var(--v-theme-on-surface), var(--v-medium-emphasis-opacity));
  cursor: pointer;
  font-size: 0.8125rem;
  line-height: 1.45;
  margin-block-start: 4px;
  max-block-size: calc(0.8125rem * 1.45 * 3);
  text-align: start;
  white-space: pre-wrap;
}

.mp-notification-text--expanded {
  max-block-size: none;
  overflow: visible;
}

.mp-notification-meta {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  color: rgba(var(--v-theme-on-surface), var(--v-disabled-opacity));
  font-size: 0.75rem;
  gap: 6px;
  line-height: 1.2;
  margin-block-start: 6px;
}

.mp-notification-type {
  border-radius: 999px;
  background: rgba(var(--v-theme-primary), 0.1);
  color: rgb(var(--v-theme-primary));
  padding-block: 2px;
  padding-inline: 6px;
}

.mp-notification-empty {
  color: rgba(var(--v-theme-on-surface), var(--v-medium-emphasis-opacity));
  padding-block: 32px;
  padding-inline: 16px;
  text-align: center;
}

.mp-notification-empty__icon {
  display: inline-grid;
  place-items: center;
  border-radius: 8px;
  background: rgba(var(--v-theme-on-surface), 0.06);
  block-size: 40px;
  inline-size: 40px;
  margin-block-end: 12px;
}

</style>
