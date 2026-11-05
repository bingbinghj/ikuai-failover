<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import ConfigField from './ConfigField.vue'
import { useBrowserPush } from './useBrowserPush'
import { moviePilotProviderFields, providerFields, type Field, type MoviePilotField } from './configSchema'

type Obj = Record<string, unknown>
const props = defineProps<{ channel: Obj; secrets: Obj; showPasswords?: boolean; credentialsEnabled?: boolean; request?: (path: string, init?: RequestInit) => Promise<unknown> }>()
const emit = defineEmits<{
  patch: [value: Obj]
  secret: [key: string, value: string]
  invalid: [key: string, message: string]
  'credentials-saved': []
}>()
const provider = computed(() => String(props.channel.provider ?? ''))
const revealed = ref<Record<string, boolean>>({})
const fieldErrors = ref<Record<string, string>>({})
watch(() => props.channel.tag, () => { revealed.value = {} })
const voceTarget = computed(() => String(props.channel.target ?? ''))
const schema = computed(() => (providerFields[provider.value] ?? []).filter(field => provider.value !== 'qy_wechat_bot' || field.key !== 'message_type'))
const mpConfig = computed(() => object(props.channel.mp_config))
const mpSchema = computed(() => (moviePilotProviderFields[provider.value] ?? []).filter(field => !field.mode || field.mode === String(mpConfig.value.WECHAT_MODE ?? 'app')))
const compatibilityFields = computed(() => schema.value.filter(field => provider.value === 'wechat_claw_bot' ? ['bot_token_env', 'account_id', 'context_token_env'].includes(field.key) : provider.value === 'app_push' ? ['owner', 'route'].includes(field.key) : false))
const { supported: pushSupported, busy: pushBusy, enabled: pushEnabled, status: pushStatus, message: pushMessage, enable: enablePush } = useBrowserPush((path, init) => props.request ? props.request(path, init) : Promise.reject(new Error('请在通知设置中开启浏览器推送。')), () => String(props.channel.tag), () => String(mpConfig.value.WEBPUSH_USERNAME ?? ''))
async function enableBrowserPush() { if (props.credentialsEnabled !== false && await enablePush()) emit('credentials-saved') }
watch(() => mpConfig.value.WECHAT_MODE, () => {
  const visible = new Set(mpSchema.value.map(field => field.key))
  for (const key of Object.keys(fieldErrors.value)) if (!visible.has(key)) { fieldErrors.value[key] = ''; emit('invalid', key, '') }
})
const bindings: Record<string, { key: string; secret?: boolean }> = {
  TELEGRAM_TOKEN: { key: 'bot_token_env', secret: true }, TELEGRAM_CHAT_ID: { key: 'chat_id' }, TELEGRAM_TOPIC_ID: { key: 'message_thread_id' }, API_URL: { key: 'api_url' },
  WECHAT_CORPID: { key: 'corp_id' }, WECHAT_APP_ID: { key: 'agent_id' }, WECHAT_APP_SECRET: { key: 'corp_secret_env', secret: true }, WECHAT_PROXY: { key: 'api_url' },
  SLACK_OAUTH_TOKEN: { key: 'bot_token_env', secret: true }, SLACK_CHANNEL: { key: 'channel' }, DISCORD_BOT_TOKEN: { key: 'bot_token_env', secret: true }, DISCORD_CHANNEL_ID: { key: 'channel_id' },
  DINGTALK_WEBHOOK: { key: 'webhook_url_env', secret: true }, DINGTALK_SECRET: { key: 'secret_env', secret: true }, SYNOLOGYCHAT_WEBHOOK: { key: 'webhook_url_env', secret: true },
  VOCECHAT_HOST: { key: 'api_url' }, VOCECHAT_API_KEY: { key: 'api_key_env', secret: true }, VOCECHAT_CHANNEL_ID: { key: 'target' },
  FEISHU_APP_ID: { key: 'app_id' }, FEISHU_APP_SECRET: { key: 'app_secret_env', secret: true }, QQ_APP_ID: { key: 'app_id' }, QQ_APP_SECRET: { key: 'app_secret_env', secret: true },
  WECHATCLAWBOT_BASE_URL: { key: 'api_url' }, WECHATCLAWBOT_DEFAULT_TARGET: { key: 'recipient' }, PUSH_SERVER_URL: { key: 'api_url' }, PUSH_RELAY_KEY: { key: 'relay_key_env', secret: true },
}
function sensitive(field: MoviePilotField) { return field.kind === 'password' || bindings[field.key]?.secret || /SECRET|TOKEN|KEY|WEBHOOK/.test(field.key) }
function object(value: unknown): Obj { return value !== null && typeof value === 'object' && !Array.isArray(value) ? value as Obj : {} }
function mpValue(field: MoviePilotField): unknown {
  if (field.key in mpConfig.value) return mpConfig.value[field.key]
  if (field.key === 'WECHAT_MODE') return 'app'
  if (field.key === 'FEISHU_OPEN_ID') return props.channel.receive_id_type === 'open_id' ? props.channel.receive_id : ''
  if (field.key === 'FEISHU_CHAT_ID') return (props.channel.receive_id_type ?? 'chat_id') === 'chat_id' ? props.channel.receive_id : ''
  if (field.key === 'QQ_OPENID') return (props.channel.recipient_type ?? 'user') === 'user' ? props.channel.recipient : ''
  if (field.key === 'QQ_GROUP_OPENID') return props.channel.recipient_type === 'group' ? props.channel.recipient : ''
  const binding = bindings[field.key]
  if (!binding) return field.default ?? ''
  if (binding.secret) return props.secrets[String(props.channel[binding.key] ?? '')] ?? ''
  const value = props.channel[binding.key]
  if (field.key === 'VOCECHAT_CHANNEL_ID') return String(value ?? '').replace(/^GID#/, '')
  return value ?? field.default ?? ''
}
function writeMp(field: MoviePilotField, value: unknown) {
  let next: unknown = value ?? ''
  if (field.key === 'WECHAT_MODE') next = value ? 'bot' : 'app'
  if (field.kind === 'number') next = value === '' || value === null ? field.default ?? '' : Number(value)
  if ((field.kind === 'number' || field.key === 'WECHAT_APP_ID' || field.key === 'TELEGRAM_TOPIC_ID') && next !== '' && (!Number.isSafeInteger(Number(next)) || Number(next) < 0)) {
    fieldErrors.value[field.key] = '请填写有效的非负整数。'
    emit('invalid', field.key, `${field.label}：${fieldErrors.value[field.key]}`)
    return
  }
  fieldErrors.value[field.key] = ''
  const config: Obj = { ...mpConfig.value, [field.key]: next }
  const patch: Obj = { mp_config: config }
  const binding = bindings[field.key]
  if (binding) {
    if (binding.secret) {
      const key = String(props.channel[binding.key] ?? '').trim() || `NOTIFY_${String(props.channel.tag).toUpperCase()}_${binding.key.replace(/_env$/, '').toUpperCase()}`
      patch[binding.key] = key
      emit('secret', key, String(next))
    } else if (field.key === 'VOCECHAT_CHANNEL_ID') patch[binding.key] = next === '' ? '' : `GID#${String(next)}`
    else if (field.key === 'WECHAT_APP_ID') patch[binding.key] = Number(next)
    else if (field.key === 'TELEGRAM_TOPIC_ID') patch[binding.key] = next === '' ? undefined : Number(next)
    else if ((field.key === 'API_URL' || field.key === 'WECHAT_PROXY') && !next) patch[binding.key] = providerFields[provider.value]?.find(item => item.key === binding.key)?.default
    else patch[binding.key] = next
  }
  if (field.key === 'FEISHU_OPEN_ID' || field.key === 'FEISHU_CHAT_ID') {
    if (next || !props.channel.receive_id) { patch.receive_id = next; patch.receive_id_type = field.key === 'FEISHU_OPEN_ID' ? 'open_id' : 'chat_id' }
  }
  if (field.key === 'QQ_OPENID' || field.key === 'QQ_GROUP_OPENID') {
    if (next || !props.channel.recipient) { patch.recipient = next; patch.recipient_type = field.key === 'QQ_OPENID' ? 'user' : 'group' }
  }
  emit('patch', patch)
  emit('invalid', field.key, '')
}
function id(field: Field) { return `notify-${String(props.channel.tag)}-${field.key}` }
function isSecret(field: Field) { return field.key.endsWith('_env') }
function reference(field: Field) {
  return String(props.channel[field.key] ?? '').trim() || `NOTIFY_${String(props.channel.tag ?? provider.value).replace(/[^a-zA-Z0-9_]/g, '_').toUpperCase()}_${field.key.replace(/_env$/, '').toUpperCase()}`
}
function inputField(field: Field): Field {
  return isSecret(field) ? { ...field, default: undefined, kind: 'password' } : field
}
function value(field: Field): unknown {
  if (isSecret(field)) return props.secrets[reference(field)] ?? ''
  if (provider.value === 'voce_chat' && field.key === 'target') return voceTarget.value.replace(/^(?:GID|UID)#/, '')
  return props.channel[field.key] ?? field.default
}
function update(field: Field, next: unknown) {
  if (isSecret(field)) {
    const key = reference(field)
    if (props.channel[field.key] !== key) emit('patch', { [field.key]: key })
    emit('secret', key, typeof next === 'string' ? next : '')
    return
  }
  if (provider.value === 'voce_chat' && field.key === 'target') {
    const text = String(next ?? '').trim()
    emit('patch', { target: text ? `GID#${text}` : '' })
    return
  }
  emit('patch', { [field.key]: next })
}

</script>

<template>
  <VRow>
    <VCol cols="12" md="6"><VTextField :id="`notify-${String(channel.tag)}-name`" label="名称" :model-value="String(channel.name ?? '')" hint="通知渠道名称" persistent-hint prepend-inner-icon="mdi-label" @update:model-value="emit('patch', { name: $event })" /></VCol>
    <template v-if="mpSchema.length">
      <VCol v-for="field in mpSchema" :key="field.key" cols="12" md="6">
        <VSwitch v-if="field.key === 'WECHAT_MODE'" :model-value="mpValue(field) === 'bot'" :label="field.label" :hint="field.hint" persistent-hint @update:model-value="writeMp(field, $event)" />
        <VTextField v-else :id="`notify-${String(channel.tag)}-${field.key}`" :model-value="mpValue(field)" :label="field.label" :prepend-inner-icon="sensitive(field) ? 'mdi-key-outline' : /ADMINS|USERS/.test(field.key) ? 'mdi-account-lock-outline' : /URL|HOST|PROXY/.test(field.key) ? 'mdi-link-variant' : /ID/.test(field.key) ? 'mdi-identifier' : 'mdi-cog-outline'" :hint="field.hint" :error-messages="fieldErrors[field.key]" persistent-hint :type="sensitive(field) && !revealed[field.key] ? 'password' : field.kind === 'number' ? 'number' : 'text'" :autocomplete="sensitive(field) ? 'new-password' : 'off'" @update:model-value="writeMp(field, $event)"><template v-if="sensitive(field)" #append-inner><VBtn :icon="revealed[field.key] ? 'mdi-eye-off-outline' : 'mdi-eye-outline'" variant="text" color="default" density="compact" :aria-label="revealed[field.key] ? '隐藏此凭据' : '显示此凭据'" :aria-pressed="Boolean(revealed[field.key])" @mousedown.prevent @click.stop="revealed[field.key] = !revealed[field.key]" /></template></VTextField>
      </VCol>
    </template>
    <template v-else>
      <VCol v-for="field in schema" :key="field.key" cols="12" md="6"><ConfigField :field="inputField(field)" :id="id(field)" :value="value(field)" :show-passwords="showPasswords" @update="update(field, $event)" @invalid="emit('invalid', field.key, $event)" /></VCol>
    </template>
    <template v-if="compatibilityFields.length"><VCol cols="12"><VDivider class="my-2" /><div class="text-subtitle-1">已有接入凭据</div></VCol><VCol v-for="field in compatibilityFields" :key="field.key" cols="12" md="6"><ConfigField :field="inputField(field)" :id="id(field)" :value="value(field)" :show-passwords="showPasswords" @update="update(field, $event)" @invalid="emit('invalid', field.key, $event)" /></VCol></template>
    <VCol v-if="provider === 'web_push'" cols="12">
      <VAlert v-if="!pushSupported" type="warning" variant="tonal" text="当前浏览器不支持 WebPush 通知。" />
      <div v-else class="d-flex flex-column align-start ga-3"><VChip v-if="pushEnabled" color="success" variant="tonal">浏览器通知权限已允许</VChip><VBtn color="primary" variant="tonal" prepend-icon="mdi-bell-outline" :loading="pushBusy" :disabled="credentialsEnabled === false || !String(mpConfig.WEBPUSH_USERNAME ?? '').trim()" @click="enableBrowserPush">{{ pushEnabled ? '重新登记通知' : '开启浏览器通知' }}</VBtn><span class="text-body-2 text-medium-emphasis">点击后允许当前浏览器发送 WebPush 通知，iOS 需要在用户点击时完成授权。</span></div>
      <VAlert v-if="pushMessage" :type="pushStatus === 'error' ? 'error' : 'info'" variant="tonal" class="mt-4" :text="pushMessage" />
    </VCol>
  </VRow>
</template>

<style scoped>
</style>
