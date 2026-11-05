<script setup lang="ts">
import { computed, ref, toRaw } from 'vue'
import { useDisplay } from 'vuetify'
import NotificationChannelEditor from './NotificationChannelEditor.vue'
import ClawBotSettings from './ClawBotSettings.vue'
import { providerFields, providerLabels } from './configSchema'
import { notificationProviderIcon } from './notificationProviderIcons'
import { notificationTypes, normalizeNotificationTypes } from './notificationTypes'
import { isWechatApp, notificationDefaultFormat, notificationFormatHint, notificationFormatOptions, notificationMessageFormat } from './notificationFormats'

type Obj = Record<string, unknown>
const props = defineProps<{
  modelValue: Obj[]
  secrets: Obj
  notifications?: Obj
  request: (path: string, init?: RequestInit) => Promise<unknown>
  showPasswords?: boolean
  disabled?: boolean
  credentialsEnabled?: boolean
}>()
const emit = defineEmits<{
  'update:modelValue': [value: Obj[]]
  secret: [key: string, value: string]
  invalid: [key: string, message: string]
  refresh: []
}>()
const display = useDisplay()
const visible = ref(false)
const draft = ref<Obj>({})
const wechatApp = computed(() => isWechatApp(draft.value))
const messageFormat = computed(() => notificationMessageFormat(draft.value))
const messageFormats = computed(() => notificationFormatOptions(draft.value))
const formatHint = computed(() => notificationFormatHint(draft.value))
const secretDraft = ref<Obj>({})
const activeTag = ref('')
const draggedTag = ref('')
const errors = ref<Record<string, string>>({})
const editSession = ref(0)
const testing = ref(false)
const message = ref('')
const sent = ref(false)
let generation = 0
const label = (channel: Obj) => String(channel.name || providerLabels[String(channel.provider)] || '通知')
const logo = (channel: Obj) => notificationProviderIcon(channel.provider)
const providerOptions = computed(() => Object.entries(providerLabels).map(([value, title]) => ({ value, title, image: notificationProviderIcon(value) })))
const credentialActionsAllowed = computed(() => {
  const original = props.modelValue.find(channel => String(channel.tag) === activeTag.value)
  return props.credentialsEnabled !== false && original !== undefined && JSON.stringify(draft.value) === JSON.stringify({ ...original, switchs: normalizeNotificationTypes(original.switchs) }) && JSON.stringify(secretDraft.value) === JSON.stringify(props.secrets)
})
function unused(channel: Obj) {
  if (channel.enabled === true || String(channel.name ?? '').trim()) return false
  const schema = providerFields[String(channel.provider)]
  if (!schema) return false
  return Object.entries(channel).every(([key, value]) => {
    if (['tag', 'provider', 'enabled', 'name'].includes(key)) return true
    if (key === 'message_format') return value === notificationDefaultFormat(channel)
    if (key.endsWith('_env')) return !props.secrets[String(value)]
    const field = schema.find(item => item.key === key)
    return value === '' || value === null || value === 0 || (Array.isArray(value) && !value.length) || (field && JSON.stringify(value) === JSON.stringify(field.default))
  })
}
const channels = computed(() => props.modelValue.filter(channel => !unused(channel)))
const invalid = computed(() => Object.values(errors.value).find(Boolean) ?? '')
function open(channel: Obj) {
  if (props.disabled) return
  generation += 1
  editSession.value += 1
  activeTag.value = String(channel.tag)
  draft.value = normalizeFormat(structuredClone(toRaw(channel)))
  draft.value.switchs = normalizeNotificationTypes(draft.value.switchs)
  secretDraft.value = { ...props.secrets }
  errors.value = {}
  message.value = ''
  testing.value = false
  visible.value = true
}
function add(provider: string) {
  const existing = props.modelValue.find(channel => channel.provider === provider && unused(channel))
  if (existing) { open({ ...existing, enabled: true, name: providerLabels[provider], message_format: notificationDefaultFormat(existing) }); return }
  const tags = new Set(props.modelValue.map(channel => String(channel.tag)))
  let tag = provider, suffix = 2
  while (tags.has(tag)) tag = `${provider}_${suffix++}`
  const channel: Obj = { tag, name: providerLabels[provider], provider, enabled: true, message_format: notificationDefaultFormat({ provider }) }
  for (const field of providerFields[provider] ?? []) {
    if (field.key.endsWith('_env')) {
      channel[field.key] = `NOTIFY_${tag.toUpperCase()}_${field.key.replace(/_env$/, '').toUpperCase()}`
    } else if (!field.optional) channel[field.key] = field.default ?? (field.kind === 'number' ? 0 : '')
  }
  open(channel)
}
function close() {
  generation += 1
  visible.value = false
  testing.value = false
  emit('invalid', 'channel-dialog', '')
}
function credentialsSaved() { close(); emit('refresh') }
function normalizeFormat(channel: Obj): Obj {
  if (channel.message_format !== undefined && !notificationFormatOptions(channel).some(item => item.value === channel.message_format)) return { ...channel, message_format: notificationMessageFormat(channel) }
  return channel
}
function edit(patch: Obj) {
  draft.value = normalizeFormat({ ...draft.value, ...patch })
  for (const key of Object.keys(patch)) errors.value[key] = ''
  message.value = ''
}
function save() {
  if (invalid.value || testing.value) return
  const name = String(draft.value.name ?? '').trim()
  if (!name) { errors.value.name = '请输入名称'; return }
  const channels = props.modelValue.map(channel => String(channel.tag) === activeTag.value ? { ...draft.value, name } : channel)
  if (!channels.some(channel => String(channel.tag) === activeTag.value)) channels.push({ ...draft.value, name })
  for (const [key, value] of Object.entries(secretDraft.value)) {
    if (typeof value === 'string' && value !== props.secrets[key]) emit('secret', key, value)
  }
  emit('update:modelValue', channels)
  close()
}
function remove(tag: string) {
  emit('update:modelValue', props.modelValue.filter(channel => String(channel.tag) !== tag))
}
function reorder(target: string) {
  const from = props.modelValue.findIndex(channel => String(channel.tag) === draggedTag.value)
  const to = props.modelValue.findIndex(channel => String(channel.tag) === target)
  if (from < 0 || to < 0 || from === to) return
  const channels = [...props.modelValue]
  const channel = channels.splice(from, 1)[0]
  if (channel) { channels.splice(to, 0, channel); emit('update:modelValue', channels) }
  draggedTag.value = ''
}
function object(value: unknown): Obj { return value !== null && typeof value === 'object' && !Array.isArray(value) ? value as Obj : {} }
async function test() {
  if (testing.value || invalid.value) return
  const current = ++generation
  testing.value = true
  message.value = ''
  const channels = props.modelValue.map(channel => String(channel.tag) === activeTag.value ? draft.value : channel)
  if (!channels.some(channel => String(channel.tag) === activeTag.value)) channels.push(draft.value)
  try {
    const response = object(await props.request('/notifications/test', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ notifications: { ...props.notifications, channels, secrets: secretDraft.value }, tag: activeTag.value }) }))
    if (current !== generation) return
    sent.value = response.success === true
    message.value = typeof response.message === 'string' ? response.message : sent.value ? '测试消息已发送' : '测试发送失败，请检查配置'
  } catch (cause) {
    if (current !== generation) return
    sent.value = false
    const payload = object(object(cause).payload)
    message.value = typeof payload.message === 'string' ? payload.message : '测试发送失败，请稍后重试'
  } finally { if (current === generation) testing.value = false }
}
</script>

<template>
  <VCard class="mb-6">
    <VCardItem title="通知渠道" subtitle="设置消息发送渠道参数" />
    <VCardText>
      <slot name="before" />
      <div class="channel-grid">
        <VCard v-for="channel in channels" :key="String(channel.tag)" variant="tonal" class="channel-summary" role="button" tabindex="0" :draggable="!disabled" @dragstart="draggedTag = String(channel.tag)" @dragover.prevent @drop.prevent="reorder(String(channel.tag))" @click="open(channel)" @keydown.enter.prevent="open(channel)" @keydown.space.prevent="open(channel)">
          <div class="channel-top-actions"><VBtn icon="mdi-drag" size="small" variant="text" color="default" aria-label="拖动渠道排序" :disabled="disabled" @click.stop /><VBtn icon="mdi-close" size="small" variant="text" color="default" aria-label="删除通知渠道" :disabled="disabled" @click.stop="remove(String(channel.tag))" /></div>
          <VCardText class="channel-summary-body">
            <div class="channel-copy"><div class="channel-title"><VBadge v-if="channel.enabled" dot inline color="success" /><span class="text-h6">{{ label(channel) }}</span></div><div class="text-body-1 channel-type">{{ providerLabels[String(channel.provider)] || channel.provider }}</div></div>
            <VImg :src="logo(channel)" contain class="channel-logo" aria-hidden="true" />
          </VCardText>
        </VCard>
      </div>
      <p v-if="!channels.length" class="text-body-2 text-medium-emphasis mb-0">暂无通知渠道，点击下方添加。</p>
    </VCardText>
    <VCardText>
      <div class="d-flex flex-wrap ga-4">
        <slot name="actions" />
        <VMenu location="bottom start" :close-on-content-click="true"><template #activator="{ props: menuProps }"><VBtn v-bind="menuProps" color="success" variant="tonal" min-width="48" aria-label="添加通知渠道" :disabled="disabled || modelValue.length >= 64"><VIcon icon="mdi-plus" /></VBtn></template><VList><VListItem v-for="item in providerOptions" :key="item.value" :title="item.title" @click="add(item.value)"><template #prepend><VAvatar size="24" rounded class="mr-3"><VImg :src="item.image" contain /></VAvatar></template></VListItem></VList></VMenu>
      </div>
    </VCardText>
  </VCard>
  <VDialog :model-value="visible" scrollable :max-width="display.mdAndUp.value ? '40rem' : undefined" :fullscreen="!display.mdAndUp.value" @update:model-value="$event || close()">
    <VCard>
      <VCardItem class="py-2" title="配置" :subtitle="label(draft)"><template #prepend><VIcon icon="mdi-cog" class="mr-2" /></template><template #append><VBtn icon="mdi-close" variant="text" color="default" aria-label="关闭通知配置" @click="close" /></template></VCardItem>
      <VDivider />
      <VCardText>
        <VForm @submit.prevent="save">
          <VRow>
            <VCol cols="12" md="6"><VSwitch :model-value="draft.enabled === true" label="启用" @update:model-value="edit({ enabled: $event })" /></VCol>
            <VCol cols="12" md="6"><VSelect :model-value="messageFormat" label="消息格式" :hint="formatHint" persistent-hint :items="messageFormats" prepend-inner-icon="mdi-format-text" @update:model-value="edit(draft.provider === 'qy_wechat_bot' ? { message_format: $event, message_type: $event } : { message_format: $event })" /></VCol>
            <VCol v-if="wechatApp && messageFormat === 'markdown'" cols="12"><VSelect label="接收客户端" :model-value="object(draft.mp_config).WECHAT_APP_MARKDOWN === true" :items="[{value:false,title:'微信 / 企业微信（兼容文本）'},{value:true,title:'仅企业微信（原生 Markdown）'}]" prepend-inner-icon="mdi-cellphone" @update:model-value="edit({ mp_config: { ...object(draft.mp_config), WECHAT_APP_MARKDOWN: $event } })" /></VCol>
            <VCol cols="12"><VAutocomplete :model-value="Array.isArray(draft.switchs) ? draft.switchs : []" :items="notificationTypes" label="类型" hint="留空发送全部类型；可选择要发送的线路、策略和服务事件。" multiple clearable chips persistent-hint prepend-inner-icon="mdi-bell-outline" @update:model-value="edit({ switchs: $event })" /></VCol>
          </VRow>
          <NotificationChannelEditor :key="editSession" :channel="draft" :secrets="secretDraft" :credentials-enabled="credentialActionsAllowed" :request="request" @patch="edit" @secret="(key, value) => secretDraft[key] = value" @invalid="(key, value) => errors[key] = value" @credentials-saved="credentialsSaved" />
          <ClawBotSettings v-if="draft.provider === 'wechat_claw_bot' && credentialActionsAllowed" :tag="String(draft.tag)" :request="request" :active="visible" @credentials-saved="credentialsSaved" />
          <VAlert v-if="['wechat_claw_bot', 'web_push', 'app_push'].includes(String(draft.provider)) && !credentialActionsAllowed" type="info" variant="tonal" class="mt-4" text="请先确定当前修改并保存配置，再连接通知账户。" />
          <VAlert v-if="invalid" type="error" variant="tonal" class="mt-4" :text="invalid" />
          <VAlert v-if="message" :type="sent ? 'success' : 'error'" variant="tonal" class="mt-4" :text="message" />
        </VForm>
      </VCardText>
      <VCardActions class="pa-4"><VBtn variant="text" prepend-icon="mdi-send-outline" :loading="testing" :disabled="Boolean(invalid)" @click="test">发送测试消息</VBtn><VSpacer /><VBtn variant="flat" prepend-icon="mdi-content-save" :disabled="testing || Boolean(invalid)" @click="save">确定</VBtn></VCardActions>
    </VCard>
  </VDialog>
</template>

<style scoped>
.channel-grid{display:grid;grid-template-columns:repeat(auto-fill,minmax(min(16rem,100%),1fr));gap:12px}.channel-summary{position:relative;min-width:0}.channel-summary-body{position:relative;display:flex;align-items:stretch;min-height:120px;height:120px;padding-block:12px}.channel-copy{display:flex;flex:1;min-width:0;flex-direction:column;justify-content:space-between;padding-right:80px}.channel-title{display:flex;align-items:flex-start;gap:5px;min-width:0}.channel-title>.v-badge{flex-shrink:0;align-self:center}.channel-title>span{overflow:hidden;display:-webkit-box;-webkit-box-orient:vertical;-webkit-line-clamp:2;line-clamp:2;line-height:1.35;word-break:break-word}.channel-type{overflow:hidden;white-space:nowrap;text-overflow:ellipsis}.channel-logo{position:absolute;right:16px;bottom:12px;width:56px;height:56px;max-width:56px;max-height:56px}.channel-top-actions{position:absolute;top:4px;right:4px;z-index:2;display:flex}.channel-top-actions .v-btn{cursor:grab}
</style>
