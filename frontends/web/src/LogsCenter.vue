<script setup lang="ts">
import { nextTick, onMounted, onUnmounted, ref, watch } from 'vue'
import LogsView from './LogsView.vue'
import NotificationCenter from './NotificationCenter.vue'
import type { Notice } from './model'

type View = 'realtime' | 'history' | 'notifications'
const view = defineModel<View>({ default: 'realtime' })
const props = withDefaults(defineProps<{
  request: (path: string, init?: RequestInit) => Promise<unknown>
  notices: Notice[]
  loading: boolean
  error?: string
  unread: number
  isRead: (id: string) => boolean
  active?: boolean
  compact?: boolean
}>(), { active: true, compact: false })
const emit = defineEmits<{ read: [id: string]; readAll: []; refresh: [] }>()
const container = ref<HTMLElement | null>(null)
const height = ref<number | null>(null)
let frame = 0
function measure() {
  cancelAnimationFrame(frame)
  frame = requestAnimationFrame(() => {
    if (props.compact || !container.value) return
    const top = container.value.getBoundingClientRect().top
    height.value = Math.max(0, window.innerHeight - top - 24)
  })
}
watch(() => props.active, async active => { if (active) { await nextTick(); measure() } })
watch(() => props.compact, async () => { await nextTick(); measure() })
onMounted(() => { measure(); window.addEventListener('resize', measure) })
onUnmounted(() => { cancelAnimationFrame(frame); window.removeEventListener('resize', measure) })
</script>

<template>
  <div ref="container" class="mp-logs-center" :class="{ compact }" :style="{ blockSize: compact ? '100%' : height === null ? undefined : height + 'px' }">
    <VCard class="mp-logs-card" variant="flat">
      <VTabs v-model="view" class="mp-logs-tabs" color="primary" show-arrows>
        <VTab value="realtime" prepend-icon="mdi-console-line">实时日志</VTab>
        <VTab value="history" prepend-icon="mdi-history">历史日志</VTab>
        <VTab value="notifications" prepend-icon="mdi-bell-outline">通知记录<VChip v-if="unread" color="primary" size="x-small" class="ml-2">{{ unread > 99 ? '99+' : unread }}</VChip></VTab>
      </VTabs>
      <VDivider />
      <div class="mp-logs-panel" role="tabpanel">
        <LogsView v-if="view !== 'notifications'" :key="view" :mode="view" :request="request" :active="active" :compact="compact" />
        <NotificationCenter v-else :compact="compact" :notices="notices" :loading="loading" :error="error" :unread="unread" :is-read="isRead" @read="emit('read', $event)" @read-all="emit('readAll')" @refresh="emit('refresh')" @logs="view = 'realtime'" />
      </div>
    </VCard>
  </div>
</template>

<style scoped>
.mp-logs-center { display:flex; flex-direction:column; min-block-size:0; min-inline-size:0; overflow:hidden; }
.mp-logs-center.compact { flex:1 1 auto; block-size:100%; }
.mp-logs-card { display:flex; flex-direction:column; flex:1 1 0%; block-size:0!important; min-block-size:0; max-block-size:100%; overflow:hidden; }
.mp-logs-tabs { flex:none; }
.mp-logs-tabs :deep(.v-tab) { font-size:0.875rem; letter-spacing:0; text-transform:none; }
.mp-logs-panel { display:flex; flex-direction:column; flex:1 1 0%; min-block-size:0; min-inline-size:0; overflow:hidden; }
.compact .mp-logs-tabs :deep(.v-tab) { min-inline-size:5.5rem; padding-inline:0.75rem; font-size:0.8125rem; }
@media (width <= 640px) { .mp-logs-tabs :deep(.v-tab) { min-inline-size:5.5rem; padding-inline:0.75rem; font-size:0.8125rem; } }
</style>
