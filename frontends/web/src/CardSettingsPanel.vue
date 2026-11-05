<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from 'vue'
import { useDisplay } from 'vuetify'
import { isMap, isScalar, parseDocument, type Document } from 'yaml'
import ConfigField from './ConfigField.vue'
import NotificationChannelSettings from './NotificationChannelSettings.vue'
import type { Field } from './configSchema'

type Kind = 'connection' | 'lines' | 'rules' | 'ipv6' | 'notifications' | 'timing'
type Obj = Record<string, unknown>
type Path = (string | number)[]
type Side = 'primary' | 'backup'
type Resource = 'five_tuple_rules' | 'load_balance_rules' | 'domain_rule_state'
interface InterfaceOption { name: string; label: string; kind: string; default_route: boolean }
interface RuleOption { id: number; tag: string; interface: string; label: string; selector_fingerprint: string; selectable: boolean; reason: string }
interface Ipv6Option { id: number; tag: string; interface: string; label: string; selectable: boolean; reason: string }
interface Options { interfaces: InterfaceOption[]; rules: Record<Resource, RuleOption[]>; ipv6: { wan: Ipv6Option[]; lan: Ipv6Option[] } }
const props = defineProps<{ kind: Kind; line?: string; request: (path: string, init?: RequestInit) => Promise<unknown> }>()
const emit = defineEmits<{ saved: []; restart: [] }>()
const { mdAndUp } = useDisplay()
const source = ref(''), savedSource = ref(''), revision = ref(''), error = ref(''), success = ref('')
const loading = ref(true), saving = ref(false), discovering = ref(false), restartOpen = ref(false)
const options = ref<Options | null>(null), fieldErrors = ref<Record<string, string>>({})
const selectedResource = ref<Resource>('five_tuple_rules')
let alive = true, loadGeneration = 0
function object(value: unknown): Obj { return value !== null && typeof value === 'object' && !Array.isArray(value) ? value as Obj : {} }
const parsed = computed(() => {
  try { const doc = parseDocument(source.value, { uniqueKeys: true, prettyErrors: false }); if (doc.errors.length) return null; return { doc: doc as Document, config: object(doc.toJS({ maxAliasCount: 40 })) } }
  catch { return null }
})
function at(path: Path): unknown { let value: unknown = parsed.value?.config; for (const key of path) { if (Array.isArray(value)) value = value[Number(key)]; else value = object(value)[String(key)] } return value }
function mutate(update: (doc: Document) => void) { const doc = parsed.value?.doc.clone(); if (!doc) return; try { update(doc); source.value = doc.toString({ lineWidth: 0 }); success.value = ''; error.value = '' } catch { error.value = '配置格式无法编辑，请检查部署目录中的配置文件。' } }
function assign(doc: Document, path: Path, value: unknown) {
  for (let length = 1; length < path.length; length++) {
    const prefix = path.slice(0, length), node = doc.getIn(prefix, true)
    if (node === undefined || node === null || (isScalar(node) && node.value === null)) doc.setIn(prefix, doc.createNode(typeof path[length] === 'number' ? [] : {}))
  }
  if (value === undefined) doc.deleteIn(path); else doc.setIn(path, doc.createNode(value))
}
function put(path: Path, value: unknown) { mutate(doc => assign(doc, path, value)) }
function canonical(value: unknown): unknown { if (Array.isArray(value)) return value.map(canonical); if (value !== null && typeof value === 'object') return Object.fromEntries(Object.entries(object(value)).sort(([left], [right]) => left.localeCompare(right)).map(([key, item]) => [key, canonical(item)])); return value }
const dirty = computed(() => { try { return JSON.stringify(canonical(parsed.value?.config)) !== JSON.stringify(canonical(parseDocument(savedSource.value).toJS({ maxAliasCount: 40 }))) } catch { return source.value !== savedSource.value } })
const canSave = computed(() => Boolean(revision.value && parsed.value && dirty.value && !loading.value && !saving.value && !Object.values(fieldErrors.value).some(Boolean) && !panelIssue.value))
const resources = [{ value: 'five_tuple_rules', title: '防回环' }, { value: 'load_balance_rules', title: '运营商分流' }, { value: 'domain_rule_state', title: '域名分流' }]
const lineRoles = [{ key: 'primary', label: '主线路' }, { key: 'backup', label: '备用线路' }, { key: 'preserved_default', label: '旁路由出口' }]
const editingLineRole = ref('')
const shownLineRoles = computed(() => props.line ? lineRoles.filter(item => item.key === editingLineRole.value) : lineRoles)
function interfaceItems(key: string) { return (options.value?.interfaces ?? []).filter(item => item.kind === 'wan').map(item => ({ value: item.name, title: item.label, props: { disabled: lineRoles.some(role => role.key !== key && at(['wan', role.key]) === item.name) || (key === 'preserved_default' ? !item.default_route : item.default_route) } })) }
function interfaceName(value: unknown) { const name = String(value ?? ''); return options.value?.interfaces.find(item => item.name === name)?.label ?? name.toUpperCase() }
const pairs = computed(() => Array.isArray(at(['wan', 'rule_pairs'])) ? (at(['wan', 'rule_pairs']) as unknown[]).map(object) : [])
function pairRules(index: number, side: Side) { const resource = String(pairs.value[index]?.resource) as Resource; return (options.value?.rules[resource] ?? []).filter(rule => rule.interface === at(['wan', side])) }
function selectedRule(index: number, side: Side) { return pairRules(index, side).find(rule => rule.id === pairs.value[index]?.[`${side}_id`] && rule.tag === pairs.value[index]?.[`${side}_tag`]) }
function usedRule(index: number, rule: RuleOption) { return pairs.value.some((pair, other) => other !== index && pair.resource === pairs.value[index]?.resource && (pair.primary_id === rule.id || pair.backup_id === rule.id)) }
function ruleDisabled(index: number, side: Side, rule: RuleOption) { const primary = selectedRule(index, 'primary'); return !rule.selectable || usedRule(index, rule) || (side === 'backup' && (!primary || !primary.selector_fingerprint || primary.selector_fingerprint !== rule.selector_fingerprint)) }
function ruleItems(index: number, side: Side) { return pairRules(index, side).map(rule => ({ value: `${rule.id}:${rule.tag}`, title: rule.label, props: { disabled: ruleDisabled(index, side, rule) } })) }
function ruleValue(index: number, side: Side) { const pair = pairs.value[index]; return pair?.[`${side}_id`] ? `${pair[`${side}_id`]}:${pair[`${side}_tag`]}` : '' }
function chooseRule(index: number, side: Side, value: unknown) {
 const rule = pairRules(index, side).find(item => `${item.id}:${item.tag}` === value); if (!rule || ruleDisabled(index, side, rule)) return
 mutate(doc => {
  assign(doc, ['wan', 'rule_pairs', index, `${side}_id`], rule.id); assign(doc, ['wan', 'rule_pairs', index, `${side}_tag`], rule.tag)
  if (side === 'primary') {
   const previous = selectedRule(index, 'backup')
   if (previous?.selectable && previous.selector_fingerprint === rule.selector_fingerprint) return
   const matches = pairRules(index, 'backup').filter(item => item.selectable && !usedRule(index, item) && item.selector_fingerprint === rule.selector_fingerprint)
   const backup = matches.length === 1 ? matches[0] : undefined
   assign(doc, ['wan', 'rule_pairs', index, 'backup_id'], backup?.id ?? 0); assign(doc, ['wan', 'rule_pairs', index, 'backup_tag'], backup?.tag ?? '')
  }
 })
}
const panelIssue = computed(() => {
 if (props.kind === 'rules') {
  for (let index = 0; index < pairs.value.length; index++) {
   const primary = selectedRule(index, 'primary'), backup = selectedRule(index, 'backup')
   if (!primary || !backup) return '请选择每组规则的主线路与备用线路。'
   if (!primary.selectable || !backup.selectable || !primary.selector_fingerprint || primary.selector_fingerprint !== backup.selector_fingerprint) return '主备规则需要匹配相同流量。'
  }
 }
 if (props.kind === 'ipv6' && at(['wan', 'ipv6_enabled']) === true && (!at(['wan', 'ipv6', 'wan_id']) || !at(['wan', 'ipv6', 'lan_id']) || !at(['wan', 'ipv6', 'lan_interface']))) return '请选择 IPv6 出口与内网配置。'
 return ''
})
function addPair() { put(['wan', 'rule_pairs'], [...pairs.value, { resource: selectedResource.value, primary_id: 0, backup_id: 0, primary_tag: '', backup_tag: '' }]) }
function removePair(index: number) { put(['wan', 'rule_pairs'], pairs.value.filter((_, other) => other !== index)); fieldErrors.value = {} }
function changeResource(index: number, resource: unknown) { put(['wan', 'rule_pairs', index], { resource, primary_id: 0, backup_id: 0, primary_tag: '', backup_tag: '' }) }
function ipv6Items(side: 'wan' | 'lan') { return (options.value?.ipv6[side] ?? []).map(item => ({ value: `${item.id}:${item.tag}`, title: item.label, props: { disabled: !item.selectable } })) }
function ipv6Value(side: 'wan' | 'lan') { const value = at(['wan', 'ipv6', `${side}_id`]); return value ? `${value}:${at(['wan', 'ipv6', `${side}_tag`])}` : '' }
function chooseIpv6(side: 'wan' | 'lan', value: unknown) {
 const option = options.value?.ipv6[side].find(item => `${item.id}:${item.tag}` === value); if (!option?.selectable) return
 mutate(doc => { const profile = doc.getIn(['wan', 'ipv6'], true); if (!isMap(profile)) assign(doc, ['wan', 'ipv6'], { settle_timeout_ms: 60000, poll_interval_ms: 1000 }); assign(doc, ['wan', 'ipv6', `${side}_id`], option.id); assign(doc, ['wan', 'ipv6', `${side}_tag`], option.tag); if (side === 'lan') assign(doc, ['wan', 'ipv6', 'lan_interface'], option.interface) })
}
const notifications = computed(() => object(at(['notifications'])))
const channels = computed(() => Array.isArray(at(['notifications', 'channels'])) ? (at(['notifications', 'channels']) as unknown[]).map(object) : [])
function failure(reason: unknown) { const issue = object(reason), payload = object(issue.payload); return typeof payload.message === 'string' ? payload.message : issue.status === 409 ? '配置已改变，请关闭弹窗后重新打开。' : issue.status === 401 ? '登录已过期，请重新登录。' : '无法读取或保存配置，请稍后重试。' }
async function discover() {
 if (discovering.value) return
 discovering.value = true; error.value = ''
 try { const result = object(await props.request('/settings/options', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(object(at(['ikuai']))) })); if (alive && Array.isArray(result.interfaces) && result.rules && result.ipv6) options.value = result as unknown as Options }
 catch (cause) { if (alive) error.value = failure(cause) }
 finally { if (alive) discovering.value = false }
}
async function load(): Promise<boolean> {
 const version = ++loadGeneration
 loading.value = true; error.value = ''; success.value = ''; fieldErrors.value = {}
 try {
  const data = object(await props.request('/settings/config'))
  if (!alive || version !== loadGeneration) return false
  if (typeof data.yaml !== 'string' || typeof data.revision !== 'string') { error.value = '配置文件无法读取。'; return false }
  source.value = savedSource.value = data.yaml; revision.value = data.revision
  if (props.line && !editingLineRole.value) editingLineRole.value = lineRoles.find(item => at(['wan', item.key]) === props.line)?.key ?? ''
  try { if (['lines', 'rules', 'ipv6'].includes(props.kind)) { const result = object(await props.request('/settings/options'));  if (alive && version === loadGeneration && Array.isArray(result.interfaces) && result.rules && result.ipv6) options.value = result as unknown as Options } } catch { /* The dedicated form stays usable before router credentials are configured. */ }
  return alive && version === loadGeneration
 } catch (cause) { if (alive && version === loadGeneration) error.value = failure(cause); return false }
 finally { if (alive && version === loadGeneration) loading.value = false }
}
async function refreshCredentials() {
 if (dirty.value || Object.values(fieldErrors.value).some(Boolean)) { fieldErrors.value['credentials-stale'] = '通知凭据已更新，当前输入已保留。请关闭弹窗后重新打开，再继续保存。'; return }
 if (await load() && alive) restartOpen.value = true
}

async function save() {
 if (!canSave.value) return
 const document = parseDocument(savedSource.value, { uniqueKeys: true, prettyErrors: false })
 const paths: Path[] = props.kind === 'connection' ? [['ikuai']] : props.kind === 'lines' ? [...shownLineRoles.value.map(item => ['wan', item.key]), ...(!props.line ? [['management', 'apply']] : [])] : props.kind === 'rules' ? [['wan', 'rule_pairs']] : props.kind === 'ipv6' ? [['wan', 'ipv6_enabled'], ['wan', 'ipv6']] : props.kind === 'notifications' ? [['notifications', 'enabled'], ['notifications', 'channels'], ['notifications', 'secrets']] : [['wan', 'check_interval_ms'], ['wan', 'failure_threshold'], ['wan', 'recovery_threshold'], ['wan', 'stable_hold_ms']]
 for (const path of paths) assign(document as Document, path, at(path))
 const yaml = document.toString({ lineWidth: 0 }); saving.value = true; error.value = ''; success.value = ''
 try { const result = object(await props.request('/settings/config', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ yaml, revision: revision.value }) })); if (!alive) return; if (result.success !== true || typeof result.revision !== 'string') throw new Error(); savedSource.value = yaml; revision.value = result.revision; success.value = '配置已保存。'; emit('saved'); restartOpen.value = result.restart_required === true }
 catch (cause) { if (alive) error.value = failure(cause) }
 finally { if (alive) saving.value = false }
}
function invalid(key: string, message: string) { fieldErrors.value[key] = message }
const connectionFields: Field[] = [{ key: 'base_url', label: '爱快管理地址' }, { key: 'username', label: '爱快账号' }, { key: 'password', label: '爱快密码', kind: 'password' }, { key: 'allow_invalid_certificate', label: '允许自签名证书', kind: 'boolean', default: true }, { key: 'timeout_ms', label: '连接超时（毫秒）', kind: 'number', default: 5000, min: 1, max: 30000 }]
const timingFields: Field[] = [{ key: 'check_interval_ms', label: '检查间隔（毫秒）', kind: 'number', default: 5000, min: 100 }, { key: 'failure_threshold', label: '连续失败几次后切换', kind: 'number', default: 3, min: 1 }, { key: 'recovery_threshold', label: '连续正常几次后切回', kind: 'number', default: 5, min: 1 }, { key: 'stable_hold_ms', label: '切换后至少等待多久（毫秒）', kind: 'number', default: 30000, min: 0 }]
const ipv6Fields: Field[] = [{ key: 'settle_timeout_ms', label: '等待 IPv6 地址与前缀（毫秒）', kind: 'number', default: 60000, min: 100, max: 90000 }, { key: 'poll_interval_ms', label: '等待期间检查间隔（毫秒）', kind: 'number', default: 1000, min: 10, max: 5000 }]
defineExpose({ save, canSave, dirty, saving })
onMounted(() => { void load() })
onUnmounted(() => { alive = false; loadGeneration += 1 })
</script>
<template>
 <section class="card-settings-panel">
  <VProgressLinear v-if="loading" color="primary" indeterminate class="mb-5" />
  <VAlert v-if="error" type="error" variant="tonal" class="mb-5">{{ error }}</VAlert><VAlert v-if="success" type="success" variant="tonal" class="mb-5">{{ success }}</VAlert>
  <template v-if="!loading && parsed"><VAlert v-if="dirty && panelIssue" type="info" variant="tonal" class="mb-5">{{ panelIssue }}</VAlert>
   <template v-if="kind === 'connection'"><VRow><VCol v-for="field in connectionFields" :key="field.key" cols="12" :md="field.kind === 'boolean' ? 12 : 6"><ConfigField :field="field" :value="at(['ikuai', field.key])" :id="`card-connection-${field.key}`" @update="put(['ikuai', field.key], $event)" @invalid="invalid(field.key, $event)" /></VCol></VRow><VBtn variant="tonal" :loading="discovering" prepend-icon="mdi-connection" @click="discover">连接爱快</VBtn></template>
   <template v-else-if="kind === 'lines'"><VRow><VCol v-for="item in shownLineRoles" :key="item.key" cols="12"><VSelect prepend-inner-icon="mdi-router-wireless" :label="item.label" :model-value="String(at(['wan', item.key]) ?? '')" :items="interfaceItems(item.key)" :disabled="!options" @update:model-value="put(['wan', item.key], $event)" /></VCol><VCol v-if="!line" cols="12"><VSwitch :model-value="at(['management', 'apply']) === true" label="启用故障转移" color="primary" inset hide-details @update:model-value="put(['management', 'apply'], $event)" /></VCol></VRow></template>
   <template v-else-if="kind === 'rules'"><div class="rule-add"><VSelect prepend-inner-icon="mdi-router-wireless" v-model="selectedResource" :items="resources" label="规则类型" hide-details /><VBtn :disabled="!options" prepend-icon="mdi-plus" variant="tonal" @click="addPair">添加</VBtn></div><VCard v-for="(pair, index) in pairs" :key="index" variant="outlined" class="mt-4"><VCardItem :title="resources.find(item => item.value === pair.resource)?.title ?? '分流规则'"><template #append><VBtn icon="mdi-delete-outline" variant="text" color="error" aria-label="移除规则" @click="removePair(index)" /></template></VCardItem><VCardText><VSelect prepend-inner-icon="mdi-router-wireless" :model-value="String(pair.resource)" :items="resources" label="规则类型" class="mb-7" @update:model-value="changeResource(index, $event)" /><VRow><VCol v-for="side in ['primary', 'backup'] as const" :key="side" cols="12" md="6"><VSelect prepend-inner-icon="mdi-router-wireless" :label="side === 'primary' ? '主线路规则' : '备用线路规则'" :model-value="ruleValue(index, side)" :items="ruleItems(index, side)" :hint="interfaceName(at(['wan', side]))" persistent-hint :disabled="!options || (side === 'backup' && !selectedRule(index, 'primary'))" @update:model-value="chooseRule(index, side, $event)" /></VCol></VRow></VCardText></VCard><VAlert v-if="!pairs.length" type="info" variant="tonal" class="mt-4">添加需要跟随出口切换的分流规则。</VAlert></template>
   <template v-else-if="kind === 'ipv6'"><VSwitch :model-value="at(['wan', 'ipv6_enabled']) === true" label="启用 IPv6 故障转移" color="primary" inset hide-details class="mb-6" @update:model-value="put(['wan', 'ipv6_enabled'], $event)" /><VRow><VCol v-for="side in ['wan', 'lan'] as const" :key="side" cols="12" md="6"><VSelect prepend-inner-icon="mdi-router-wireless" :label="side === 'wan' ? 'IPv6 出口' : 'IPv6 内网'" :model-value="ipv6Value(side)" :items="ipv6Items(side)" :disabled="!options" @update:model-value="chooseIpv6(side, $event)" /></VCol></VRow><VRow v-if="at(['wan', 'ipv6'])" class="mt-2"><VCol v-for="field in ipv6Fields" :key="field.key" cols="12" md="6"><ConfigField :field="field" :value="at(['wan', 'ipv6', field.key])" :id="`card-ipv6-${field.key}`" @update="put(['wan', 'ipv6', field.key], $event)" @invalid="invalid(field.key, $event)" /></VCol></VRow></template>
   <template v-else-if="kind === 'notifications'"><VSwitch :model-value="notifications.enabled === true" label="发送通知" color="primary" inset hide-details class="mb-6" @update:model-value="put(['notifications', 'enabled'], $event)" /><NotificationChannelSettings :model-value="channels" :secrets="object(at(['notifications', 'secrets']))" :notifications="notifications" :request="request" :credentials-enabled="!dirty && !Object.values(fieldErrors).some(Boolean)" :disabled="saving" @update:model-value="put(['notifications', 'channels'], $event)" @secret="(key, value) => put(['notifications', 'secrets', key], value)" @invalid="invalid" @refresh="refreshCredentials" /></template>
   <VRow v-else><VCol v-for="field in timingFields" :key="field.key" cols="12" md="6"><ConfigField :field="field" :value="at(['wan', field.key])" :id="`card-timing-${field.key}`" @update="put(['wan', field.key], $event)" @invalid="invalid(field.key, $event)" /></VCol></VRow>
   <VBtn v-if="!options && ['lines', 'rules', 'ipv6'].includes(kind)" variant="tonal" :loading="discovering" class="mt-5" prepend-icon="mdi-refresh" @click="discover">读取接口和规则</VBtn>
  </template>
  <VDialog v-model="restartOpen" :max-width="mdAndUp ? '26rem' : undefined" :fullscreen="!mdAndUp"><VCard><VCardItem title="配置已保存" /><VCardText>重启服务后使用新配置。</VCardText><VCardActions><VSpacer /><VBtn variant="text" @click="restartOpen = false">稍后重启</VBtn><VBtn variant="flat" @click="restartOpen = false; emit('restart')">现在重启</VBtn></VCardActions></VCard></VDialog>
 </section>
</template>
<style scoped>
.card-settings-panel{min-width:0}.rule-add{display:flex;align-items:center;gap:16px}.rule-add>.v-input{flex:1;min-width:0}@media(max-width:600px){.rule-add{gap:12px}}
</style>
