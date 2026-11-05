<script setup lang="ts">
import { computed, nextTick, onMounted, onUnmounted, ref, watch } from 'vue'
import { useDisplay } from 'vuetify'
import { notificationSamples } from './notificationTypes'
import { providerLabels } from './configSchema'
import { notificationFormatHint, notificationFormatOptions, notificationMessageFormat, type NotificationFormat } from './notificationFormats'

interface TemplateEvent { kind: string; label: string; icon?: string; color?: string; sampleKind?: string }
interface TemplateContext { title?: string; text?: string; kind?: string }
type TestFormat = 'channel' | NotificationFormat
const props = defineProps<{
  modelValue: Record<string, string>
  events: readonly TemplateEvent[]
  request: (path: string, init?: RequestInit) => Promise<unknown>
  context?: TemplateContext
  notifications?: Record<string, unknown>
}>()
const emit = defineEmits<{ 'update:modelValue': [value: Record<string, string>]; invalid: [message: string]; save: [] }>()
const visible = ref(false)
const display = useDisplay()
const textarea = ref<HTMLTextAreaElement | null>(null)
const selected = ref('default')
const content = ref('{}')
const title = ref('主线路离线，已切换备用线路')
const text = ref('主线路已连续多次检查失败，当前流量由备用线路接替。')
const error = ref('')
const busy = ref(false)
const testBusy = ref(false)
const testChannel = ref('')
const testFormat = ref<TestFormat>('channel')
const testMessage = ref('')
const testSuccess = ref(false)
const preview = ref<{ title: string; text: string } | null>(null)
const defaults = ref<Record<string, string>>({})
const defaultsError = ref('')
const restoring = ref(false)
let defaultsPending: Promise<void> | undefined
let mounted = true
let generation = 0
let testSequence = 0
const enabledTestChannels = computed(() => {
  const channels = props.notifications?.channels
  return Array.isArray(channels) ? channels.map(object).filter(channel => channel.enabled === true && typeof channel.tag === 'string' && channel.tag.length > 0) : []
})
const testChannels = computed(() => enabledTestChannels.value.map(channel => ({ value: String(channel.tag), title: String(channel.name || providerLabels[String(channel.provider)] || channel.tag) })))
const selectedTestChannel = computed(() => enabledTestChannels.value.find(channel => channel.tag === testChannel.value))
const testFormats = computed(() => [
  { value: 'channel', title: '按渠道配置' },
  ...notificationFormatOptions(selectedTestChannel.value),
])
const testFormatHint = computed(() => {
  const format = testFormat.value === 'channel' ? notificationMessageFormat(selectedTestChannel.value) : testFormat.value
  return `${testFormat.value === 'channel' ? '使用所选渠道当前的消息格式。' : ''}${notificationFormatHint(selectedTestChannel.value, format)}`
})
watch(testChannels, channels => { if (!channels.some(channel => channel.value === testChannel.value)) testChannel.value = channels[0]?.value ?? '' }, { immediate: true })
watch(testFormats, formats => { if (!formats.some(format => format.value === testFormat.value)) testFormat.value = 'channel' })
watch([testChannel, testFormat], () => { testMessage.value = ''; testSuccess.value = false })
const cards = computed(() => props.events.slice(0, 4))
const selectedLabel = computed(() => selected.value === 'default' ? '默认模板' : cards.value.find(item => item.kind === selected.value)?.label ?? selected.value)
const defaultTemplate = (kind: string) => defaults.value[kind] ?? defaults.value.default ?? '{}'
function canonicalTemplate(value: unknown): unknown {
  if (Array.isArray(value)) return value.map(canonicalTemplate)
  if (value !== null && typeof value === 'object') return Object.fromEntries(Object.entries(object(value)).sort(([left], [right]) => left.localeCompare(right)).map(([key, item]) => [key, canonicalTemplate(item)]))
  return value
}
function matchesDefault(source: string, builtin: string) {
  try {
    const current: unknown = JSON.parse(source)
    if (current !== null && typeof current === 'object' && !Array.isArray(current) && !Object.keys(current).length) return false
    const original: unknown = JSON.parse(builtin)
    return JSON.stringify(canonicalTemplate(current)) === JSON.stringify(canonicalTemplate(original))
  } catch { return source.trim() !== '{}' && source.trim() === builtin.trim() }
}
const customized = (kind: string) => Boolean(props.modelValue[kind]?.trim() && !matchesDefault(props.modelValue[kind]!, defaultTemplate(kind)))
const changed = computed(() => {
  if (restoring.value && props.modelValue[selected.value] !== undefined) return true
  const original = props.modelValue[selected.value] ?? defaultTemplate(selected.value)
  try { return JSON.stringify(canonicalTemplate(JSON.parse(content.value))) !== JSON.stringify(canonicalTemplate(JSON.parse(original))) }
  catch { return content.value.trim() !== original.trim() }
})
const previewKind = computed(() => props.context?.kind ?? cards.value.find(item => item.kind === selected.value)?.sampleKind ?? (selected.value === 'default' ? 'egress_switched' : selected.value))
async function loadDefaults() {
  if (defaultsPending) return defaultsPending
  defaultsPending = (async () => {
    try {
      const value = object(await props.request('/notifications/templates/defaults'))
      const templates = object(value.templates)
      if (!mounted) return
      defaults.value = Object.fromEntries(Object.entries(templates).filter((entry): entry is [string, string] => typeof entry[1] === 'string'))
      defaultsError.value = ''
    } catch { if (mounted) defaultsError.value = '默认模板读取失败，请稍后重试。' }
    finally { defaultsPending = undefined }
  })()
  return defaultsPending
}
async function open(kind: string) {
  if (!Object.keys(defaults.value).length) await loadDefaults()
  if (!mounted) return
  generation += 1
  testSequence += 1
  testBusy.value = false
  testFormat.value = 'channel'
  testMessage.value = ''
  selected.value = kind
  restoring.value = false
  content.value = props.modelValue[kind] ?? defaultTemplate(kind)
  const sample = notificationSamples[previewKind.value]
  title.value = props.context?.title ?? sample?.title ?? '网络通知'
  text.value = props.context?.text ?? sample?.text ?? '线路状态已更新。'
  error.value = ''
  preview.value = null
  busy.value = false
  visible.value = true
  await nextTick()
  textarea.value?.focus()
}
function close() {
  generation += 1
  testSequence += 1
  testBusy.value = false
  busy.value = false
  visible.value = false
  emit('invalid', '')
}
function edit() {
  restoring.value = false
  generation += 1
  error.value = ''
  preview.value = null
  testMessage.value = ''
  busy.value = false
  emit('invalid', '')
}
function reset() {
  content.value = defaultTemplate(selected.value)
  edit()
  restoring.value = true
}
function insertExample() {
  content.value = '{\n  "title": "{{ title }}",\n  "text": "{{ text }}"\n}'
  edit()
}
function object(value: unknown): Record<string, unknown> {
  return value !== null && typeof value === 'object' && !Array.isArray(value) ? value as Record<string, unknown> : {}
}
async function render(save = false) {
  if (busy.value || testBusy.value) return
  const source = content.value.trim() || '{}'
  if (new TextEncoder().encode(source).length > 16384) {
    error.value = '模板内容不能超过 16 KiB。'
    emit('invalid', error.value)
    return
  }
  const current = ++generation
  busy.value = true
  error.value = ''
  try {
    const response = object(await props.request('/notifications/template/preview', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ template: source, title: title.value, text: text.value, kind: previewKind.value }),
    }))
    if (current !== generation) return
    if (response.success !== true || typeof response.title !== 'string' || typeof response.text !== 'string') {
      throw new Error(typeof response.message === 'string' ? response.message : '模板无法生成标题与正文，请检查 JSON 格式和变量。')
    }
    preview.value = { title: response.title, text: response.text }
    emit('invalid', '')
    if (save) {
      const templates = { ...props.modelValue }
      if (matchesDefault(source, defaultTemplate(selected.value))) delete templates[selected.value]
      else templates[selected.value] = source
      emit('update:modelValue', templates)
      close()
      emit('save')
    }
  } catch (cause) {
    if (current !== generation) return
    const message = object(object(cause).payload).message
    error.value = typeof message === 'string' ? message : cause instanceof Error ? cause.message : '模板预览失败，请稍后重试。'
    emit('invalid', error.value)
  } finally {
    if (current === generation) busy.value = false
  }
}
async function sendTest() {
  if (busy.value || testBusy.value || !testChannel.value || !props.notifications) return
  const template = content.value.trim() || '{}'
  if (new TextEncoder().encode(template).length > 16384) { testSuccess.value = false; testMessage.value = '模板内容不能超过 16 KiB。'; return }
  const current = generation, sequence = ++testSequence
  testBusy.value = true
  testMessage.value = ''
  try {
    const result = object(await props.request('/notifications/test', {
      method: 'POST', headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ notifications: props.notifications, tag: testChannel.value, kind: previewKind.value, template, title: title.value, text: text.value, ...(testFormat.value === 'channel' ? {} : { message_format: testFormat.value }) }),
    }))
    if (!mounted || current !== generation || sequence !== testSequence) return
    testSuccess.value = result.success === true
    testMessage.value = typeof result.message === 'string' ? result.message : testSuccess.value ? '测试消息已发送。' : '测试发送失败，请检查所选渠道的配置。'
  } catch (cause) {
    if (!mounted || current !== generation || sequence !== testSequence) return
    testSuccess.value = false
    const message = object(object(cause).payload).message
    testMessage.value = typeof message === 'string' ? message : '测试发送失败，请稍后重试。'
  } finally { if (sequence === testSequence) testBusy.value = false }
}
onMounted(() => void loadDefaults())
onUnmounted(() => { mounted = false; generation += 1; testSequence += 1 })
</script>

<template>
  <VCard>
    <VCardItem title="通知模板" subtitle="设置通知模板，支持Jinja2语法。"><template #append><VBtn variant="text" size="small" prepend-icon="mdi-tune" @click="open('default')">默认模板</VBtn></template></VCardItem>
    <VCardText>
    <VAlert v-if="defaultsError" type="warning" variant="tonal" class="mb-4" :text="defaultsError"><template #append><VBtn variant="text" @click="loadDefaults">读取默认模板</VBtn></template></VAlert>
    <div class="template-grid">
      <VCard v-for="item in cards" :key="item.kind" tag="button" type="button" variant="flat" class="template-card" :style="{ '--template-accent': `rgb(var(--v-theme-${item.color ?? 'primary'}))` }" @click="open(item.kind)">
        <VAvatar :color="item.color ?? 'primary'" variant="tonal" rounded size="40"><VIcon :icon="item.icon ?? 'mdi-bell-outline'" size="22" /></VAvatar>
        <span class="template-copy"><strong>{{ item.label }}</strong><small>{{ customized(item.kind) ? '已自定义' : 'Jinja2 JSON' }}</small></span>
        <VIcon icon="mdi-chevron-right" size="18" class="text-medium-emphasis" />
      </VCard>
    </div>
    <p class="template-footnote">分类模板覆盖默认模板。每条通知的线路、运营商及地址信息由服务提供。</p>
    </VCardText>
    <VDialog :model-value="visible" :max-width="display.mdAndUp.value ? '50rem' : undefined" :fullscreen="!display.mdAndUp.value" aria-labelledby="template-title" @update:model-value="$event || close()">
      <VCard class="template-dialog">
      <VCardItem class="dialog-header" title="通知模板" :subtitle="`${selectedLabel} · Jinja2 JSON`"><template #prepend><VAvatar color="primary" variant="tonal" rounded size="40"><VIcon icon="mdi-code-json" size="22" /></VAvatar></template><template #title><h3 id="template-title">通知模板</h3></template><template #append><VBtn icon="mdi-close" variant="text" color="default" aria-label="关闭模板编辑" @click="close" /></template></VCardItem>
      <VCardText class="dialog-body">
        <label class="editor-label" for="notification-template-content">模板内容</label>
        <textarea id="notification-template-content" ref="textarea" v-model="content" class="template-code" rows="10" spellcheck="false" autocomplete="off" :disabled="busy || testBusy" @input="edit" />
        <div class="template-tools"><VBtn variant="text" size="small" :disabled="busy || testBusy" @click="insertExample">插入标题与正文</VBtn><VBtn variant="text" size="small" :disabled="busy || testBusy" @click="reset">恢复默认模板</VBtn></div>
        <p class="syntax-hint">JSON 包含 title、text 字段。支持 title、text、event_label、time、network_summary，以及前后线路 transition_summary、from_interface、to_interface、from_isp、to_isp、from_ipv4、to_ipv4、from_ipv6、to_ipv6。空数据行自动省略。<code>{}</code> 保留原始内容。</p>
        <div class="preview-inputs"><VTextField id="template-preview-title" v-model="title" label="预览标题" autocomplete="off" :disabled="busy || testBusy" @update:model-value="edit" /><VTextarea id="template-preview-text" v-model="text" label="预览正文" rows="2" auto-grow :disabled="busy || testBusy" @update:model-value="edit" /></div>
        <VAlert v-if="error" type="error" variant="tonal" class="mt-4" :text="error" />
        <VCard v-if="preview" variant="tonal" class="template-preview">
          <VCardSubtitle>消息预览</VCardSubtitle><VCardTitle>{{ preview.title }}</VCardTitle><VCardText>{{ preview.text }}</VCardText>
          <VCardText>
            <div class="template-test-controls">
              <VSelect v-model="testChannel" :items="testChannels" label="测试渠道" prepend-inner-icon="mdi-bell-outline" :disabled="testBusy || !testChannels.length" hide-details />
              <VSelect v-model="testFormat" :items="testFormats" label="测试消息格式" prepend-inner-icon="mdi-format-text" :disabled="testBusy || !testChannel" hide-details />
              <VBtn variant="flat" prepend-icon="mdi-send-outline" :loading="testBusy" :disabled="busy || !testChannel" @click="sendTest">发送测试</VBtn>
            </div>
            <p v-if="testChannel" class="text-body-2 text-medium-emphasis mt-3">{{ testFormatHint }} 仅用于本次测试，不修改渠道配置。</p>
            <p v-if="!testChannels.length" class="text-body-2 text-medium-emphasis mt-3">添加并启用通知渠道后，即可发送测试消息。</p>
            <VAlert v-if="testMessage" :type="testSuccess ? 'success' : 'error'" variant="tonal" class="mt-3" :text="testMessage" />
          </VCardText>
        </VCard>
      </VCardText>
      <VCardActions class="dialog-footer"><VSpacer /><VBtn variant="text" :loading="busy" :disabled="testBusy" @click="render()" prepend-icon="mdi-eye-outline">预览消息</VBtn><VBtn variant="flat" :disabled="busy || testBusy || !changed" @click="render(true)" prepend-icon="mdi-content-save">保存模板</VBtn></VCardActions>
      </VCard>
    </VDialog>
  </VCard>
</template>

<style scoped>
.template-test-controls{display:flex;align-items:center;gap:12px}.template-test-controls>.v-input{flex:1;min-width:0}@media(max-width:600px){.template-test-controls{flex-direction:column;align-items:stretch}.template-test-controls>.v-input{width:100%}}

.template-grid{display:grid;grid-template-columns:repeat(4,minmax(0,1fr));gap:16px}.template-card.v-card{display:flex;align-items:center;gap:14px;min-height:84px;padding:16px;text-align:left;background:rgb(var(--v-theme-surface));border:0;border-left:2px solid color-mix(in srgb,var(--template-accent) 22%,transparent);border-radius:0;box-shadow:none}.template-card.v-card:hover{background:color-mix(in srgb,var(--template-accent) 3%,rgb(var(--v-theme-surface)))}.template-copy{flex:1;min-width:0}.template-copy strong{display:block;font-size:.875rem;font-weight:500}.template-copy small{display:block;font-size:.75rem;color:rgba(var(--v-theme-on-surface),.5);margin-top:5px}.template-footnote{font-size:.75rem;line-height:1.6;color:rgba(var(--v-theme-on-surface),.5);margin:16px 0 0}.template-dialog{display:flex;flex-direction:column;max-block-size:calc(100dvh - 2rem);overflow:hidden}.dialog-header{flex:0 0 auto;border-bottom:1px solid rgba(var(--v-border-color),var(--v-border-opacity))}.dialog-header h3{font-size:1.25rem;font-weight:500;margin:0;line-height:2rem}.dialog-body{overflow:auto;min-height:0;flex:1}.editor-label{display:block;font-size:.875rem;margin-bottom:9px}.template-code{display:block;width:100%;min-height:230px;resize:vertical;background:rgb(var(--v-theme-surface));border:1px solid rgba(var(--v-border-color),var(--v-border-opacity));border-radius:8px;color:rgb(var(--v-theme-on-surface));font-family:Consolas,monospace;font-size:.875rem;line-height:1.8;padding:16px;tab-size:2}.template-code:focus{outline:0;border-color:rgb(var(--v-theme-primary))}.template-tools{display:flex;align-items:center;gap:8px;flex-wrap:wrap;margin-top:12px}.syntax-hint{font-size:.75rem;line-height:1.9;color:rgba(var(--v-theme-on-surface),.6);margin-top:10px}.preview-inputs{display:grid;grid-template-columns:1fr 1fr;gap:16px;margin-top:20px}.template-preview{margin-top:18px}.template-preview :deep(.v-card-title){white-space:normal}.template-preview :deep(.v-card-text){white-space:pre-wrap;overflow-wrap:anywhere}.dialog-footer{padding:16px 20px;border-top:1px solid rgba(var(--v-border-color),var(--v-border-opacity));flex-shrink:0}@media(max-width:1100px){.template-grid{grid-template-columns:repeat(2,minmax(0,1fr))}}@media(max-width:960px){.template-dialog{max-block-size:100dvh;height:100dvh;border-radius:0}}@media(max-width:600px){.template-grid,.preview-inputs{grid-template-columns:1fr}.template-card.v-card{min-height:76px;padding:14px}.template-code{min-height:260px}.dialog-footer{padding:16px}}
</style>
