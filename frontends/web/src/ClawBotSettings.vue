<script setup lang="ts">
import { computed, onUnmounted, ref, watch } from 'vue'
import QRCode from 'qrcode'

type Obj = Record<string, unknown>
const props = defineProps<{ tag: string; request: (path: string, init?: RequestInit) => Promise<unknown>; active?: boolean }>()
const emit = defineEmits<{ 'credentials-saved': [] }>()
const state = ref<Obj>({})
const image = ref('')
const busy = ref(false)
const error = ref('')
let generation = 0, attempts = 0
let timer: ReturnType<typeof setTimeout> | undefined
let controller: AbortController | undefined
const connected = computed(() => state.value.connected === true)
const labels: Record<string, string> = { waiting: '等待扫码', scanned: '已扫码，待确认', confirmed: '已连接', expired: '二维码已过期' }
const label = computed(() => connected.value ? '已连接' : labels[String(state.value.qrcode_status ?? 'waiting')] ?? '等待扫码')
function object(value: unknown): Obj { return value !== null && typeof value === 'object' && !Array.isArray(value) ? value as Obj : {} }
function stop() { generation += 1; if (timer) clearTimeout(timer); controller?.abort(); timer = undefined; busy.value = false }
async function update(value: Obj, current: number) {
  if (current !== generation || !props.active) return
  state.value = value
  const url = String(value.qrcode_url ?? '')
  const rendered = url ? url.startsWith('data:image/') ? url : await QRCode.toDataURL(url, { width: 220, margin: 1 }) : ''
  if (current !== generation || !props.active) return
  image.value = rendered
  if (value.restart_required === true) emit('credentials-saved')
}
async function status() {
  if (!props.active) return
  const current = ++generation
  controller = new AbortController()
  try {
    const value = object(await props.request(`/notifications/clawbot/status?tag=${encodeURIComponent(props.tag)}`, { signal: controller.signal }))
    if (current === generation) { error.value = ''; await update(value, current) }
  } catch (cause) {
    if (current !== generation) return
    const payload = object(object(cause).payload)
    error.value = typeof payload.message === 'string' ? payload.message : '请先确定并保存该通知渠道，再扫码登录。'
  }
}
async function poll() {
  if (!props.active || connected.value || !state.value.qrcode) return
  const current = generation
  controller = new AbortController()
  try {
    const value = object(await props.request('/notifications/clawbot/poll', { method: 'POST', headers: { 'Content-Type': 'application/json' }, signal: controller.signal, body: JSON.stringify({ tag: props.tag, qrcode: state.value.qrcode }) }))
    if (current !== generation) return
    attempts = 0
    await update(value, current)
    if (!connected.value && value.qrcode_status !== 'expired') timer = setTimeout(() => void poll(), 1500)
  } catch (cause) {
    if (current !== generation) return
    const payload = object(object(cause).payload)
    error.value = typeof payload.message === 'string' ? payload.message : '二维码状态查询失败，请刷新后重试。'
    attempts += 1
    if (object(cause).status !== 409 && attempts < 3) timer = setTimeout(() => void poll(), 3000)
  }
}
async function generate() {
  if (busy.value) return
  stop()
  const current = generation
  busy.value = true
  error.value = ''
  controller = new AbortController()
  try {
    const value = object(await props.request('/notifications/clawbot/qrcode', { method: 'POST', headers: { 'Content-Type': 'application/json' }, signal: controller.signal, body: JSON.stringify({ tag: props.tag }) }))
    if (current !== generation) return
    await update(value, current)
    attempts = 0
    if (!connected.value) timer = setTimeout(() => void poll(), 1500)
  } catch (cause) {
    if (current !== generation) return
    const payload = object(object(cause).payload)
    error.value = typeof payload.message === 'string' ? payload.message : '刷新微信 ClawBot 二维码失败。'
  } finally { if (current === generation) busy.value = false }
}
async function logout() {
  if (busy.value) return
  stop()
  const current = generation
  busy.value = true
  try {
    const value = object(await props.request('/notifications/clawbot/logout', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ tag: props.tag }) }))
    if (current === generation) await update(value, current)
  } catch (cause) {
    if (current !== generation) return
    const payload = object(object(cause).payload)
    error.value = typeof payload.message === 'string' ? payload.message : '微信 ClawBot 退出登录失败。'
  } finally { if (current === generation) busy.value = false }
}
watch(() => [props.tag, props.active], () => { stop(); if (props.active) void status() }, { immediate: true })
onUnmounted(stop)
</script>

<template>
  <VRow>
    <VCol cols="12"><VDivider class="my-2" /><div class="text-subtitle-1 mb-3">登录状态</div><VChip :color="connected ? 'success' : 'warning'" variant="tonal">{{ label }}</VChip></VCol>
    <VCol v-if="image && !connected" cols="12"><VImg :src="image" width="220" height="220" contain class="mx-auto" /></VCol>
    <VCol cols="12"><div class="d-flex flex-wrap ga-3"><VBtn color="primary" variant="tonal" prepend-icon="mdi-qrcode" :loading="busy" @click="generate">刷新二维码</VBtn><VBtn v-if="connected" color="error" variant="text" :disabled="busy" @click="logout">退出登录</VBtn></div><p class="text-body-2 text-medium-emphasis mt-3">使用微信扫码绑定后，状态会自动刷新。首次使用请先保存并启用该通知渠道。</p></VCol>
    <VCol v-if="state.account_id" cols="12"><span class="text-body-2">账号ID：{{ state.account_id }}</span></VCol>
    <VCol v-if="error" cols="12"><VAlert type="warning" variant="tonal" :text="error" /></VCol>
  </VRow>
</template>
