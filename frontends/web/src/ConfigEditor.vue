<script setup lang="ts">
import { computed, nextTick, onMounted, onUnmounted, ref, watch } from 'vue'
import { isMap, isScalar, parseDocument, type Document } from 'yaml'
import ConfigField from './ConfigField.vue'
import NotificationChannelSettings from './NotificationChannelSettings.vue'
import NotificationTemplateEditor from './NotificationTemplateEditor.vue'
import type { Field } from './configSchema'
import { notificationTemplateCategories, notificationTypes } from './notificationTypes'

const props = defineProps<{ request: (path: string, init?: RequestInit) => Promise<unknown>; initialGroup?: string; group?: string; embedded?: boolean }>()
const emit = defineEmits<{ saved: []; restart: []; 'update:group': [value: string] }>()
type Path = (string | number)[]
type Obj = Record<string, unknown>
type Side = 'primary' | 'backup'
type Resource = 'five_tuple_rules' | 'load_balance_rules' | 'domain_rule_state'
interface Interface { name: string; label: string; kind: string; default_route: boolean }
interface Rule { id: number; tag: string; comment: string; interface: string; label: string; selector_fingerprint: string; selectable: boolean; reason: string }
interface Ipv6 { id: number; tag: string; interface: string; parent: string; label: string; selectable: boolean; reason: string }
interface Options { interfaces: Interface[]; rules: Record<Resource, Rule[]>; ipv6: { wan: Ipv6[]; lan: Ipv6[] }; warnings: { section: string; message: string }[] }
const startingGroup = props.group ?? props.initialGroup ?? 'connection'
const group = ref(['connection', 'wan', 'ipv6', 'notifications', 'management'].includes(startingGroup) ? startingGroup : 'connection')
watch(group, value => emit('update:group', value))
watch(() => props.group ?? props.initialGroup, value => { if (value && ['connection', 'wan', 'ipv6', 'notifications', 'management'].includes(value)) group.value = value })
const draft = ref(''), savedYaml = ref(''), revision = ref(''), error = ref(''), success = ref('')
const loading = ref(false), saving = ref(false), discovering = ref(false)
const discoveryError = ref(''), discoveredConnection = ref(''), options = ref<Options | null>(null)
const invalidFields = ref<Record<string, string>>({})
const restartDialog = ref(false)
const savingArea = ref('')
let mounted = true, generation = 0, discoveryGeneration = 0
const groups = [
  { id: 'connection', label: '连接与登录', icon: 'mdi-account-key-outline' },
  { id: 'wan', label: '自动切换', icon: 'mdi-swap-horizontal' },
  { id: 'ipv6', label: 'IPv6', icon: 'mdi-ip-network' },
  { id: 'notifications', label: '通知', icon: 'mdi-bell-outline' },
  { id: 'management', label: '服务', icon: 'mdi-cog-outline' },
]
const resources: { value: Resource; label: string }[] = [
  { value: 'five_tuple_rules', label: '防回环' }, { value: 'load_balance_rules', label: '运营商分流' }, { value: 'domain_rule_state', label: '域名分流' },
]
const notificationEnabledField: Field = { key: 'enabled', label: '启用通知', kind: 'boolean', default: false }
function seconds(values: number[]) { return values.map(value => ({ value, label: value ? `${value / 1000} 秒` : '不限制' })) }
const fields: Record<string, Field[]> = {
  ikuai: [{ key: 'base_url', label: '爱快管理地址', hint: '使用爱快管理页面的 HTTPS 地址。', default: 'https://router.example' }, { key: 'username', label: '爱快管理账号' }, { key: 'password', label: '爱快管理密码', kind: 'password' }],
  connectionAdvanced: [{ key: 'allow_invalid_certificate', label: '允许爱快的自签名证书', kind: 'boolean', default: true }, { key: 'timeout_ms', label: '连接超时', kind: 'number', min: 1, max: 30000, default: 5000, choices: seconds([3000, 5000, 10000, 15000, 30000]) }],
  webui: [{ key: 'user', label: '网页登录账号', hint: '留空后重启会随机生成，并保存到配置文件。' }, { key: 'pass', label: '网页登录密码', kind: 'password', hint: '修改后重启，需要使用新密码登录。' }],
  loginTiming: [{ key: 'session_lifetime_seconds', label: '登录多久后需要重新验证', kind: 'number', min: 60, max: 31536000, default: 604800, choices: [3600, 86400, 604800].map(value => ({ value, label: value === 3600 ? '1 小时' : value === 86400 ? '1 天' : '7 天' })), hint: '未勾选记住密码时使用。' }, { key: 'remember_lifetime_seconds', label: '记住密码的有效期', kind: 'number', min: 60, max: 31536000, default: 2592000, choices: [604800, 2592000, 7776000].map(value => ({ value, label: `${value / 86400} 天` })), hint: '勾选记住密码后，浏览器在这段时间内保持登录。' }],
  timing: [{ key: 'check_interval_ms', label: '检查间隔', kind: 'number', min: 100, default: 5000, choices: seconds([1000, 3000, 5000, 10000, 30000, 60000]) }, { key: 'failure_threshold', label: '连续失败后切换', kind: 'number', min: 1, default: 3, choices: [1, 2, 3, 5, 10].map(value => ({ value, label: `${value} 次` })) }, { key: 'recovery_threshold', label: '连续正常后回切', kind: 'number', min: 1, default: 5, choices: [1, 2, 3, 5, 10, 15].map(value => ({ value, label: `${value} 次` })) }, { key: 'stable_hold_ms', label: '切换后至少等待多久', kind: 'number', min: 0, default: 30000, choices: seconds([0, 10000, 30000, 60000, 120000, 300000]) }],
  ipv6Timing: [{ key: 'settle_timeout_ms', label: '等待地址与 PD', kind: 'number', min: 100, max: 90000, default: 60000, choices: seconds([15000, 30000, 60000, 90000]) }, { key: 'poll_interval_ms', label: '等待期间检查间隔', kind: 'number', min: 10, max: 5000, default: 1000, choices: seconds([500, 1000, 2000, 5000]) }],
  notificationTiming: [{ key: 'timeout_ms', label: '发送超时', kind: 'number', min: 100, max: 60000, default: 10000, choices: seconds([5000, 10000, 15000, 30000, 60000]) }, { key: 'cooldown_ms', label: '相同通知多久内只发一次', kind: 'number', min: 0, default: 60000, choices: seconds([0, 30000, 60000, 300000, 600000]) }, { key: 'queue_capacity', label: '最多暂存多少条通知', kind: 'number', min: 1, max: 4096, default: 128, choices: [64, 128, 256, 512, 1024].map(value => ({ value, label: `${value} 条` })) }],
  management: [{ key: 'listen', label: '服务监听地址', hint: '修改端口后，需要同步修改容器端口映射。', default: '0.0.0.0:19002' }, { key: 'state_dir', label: '状态目录', hint: '保存切换记录、通知队列和登录会话。', default: '/var/lib/ikuai-failover' }, { key: 'web_dir', label: '页面目录', default: '/usr/share/ikuai-failover/web' }],
}
const templateEvents = notificationTemplateCategories
const notificationTemplates = computed(() => object(at(['notifications', 'templates'])) as Record<string, string>)
function object(value: unknown): Obj { return value !== null && typeof value === 'object' && !Array.isArray(value) ? value as Obj : {} }
const parsed = computed(() => {
  if (!draft.value) return { doc: null as Document | null, config: {} as Obj, error: '' }
  if (new TextEncoder().encode(draft.value).length > 2 * 1024 * 1024) return { doc: null, config: {}, error: '配置文件不能超过 2 MiB。' }
  try {
    const doc = parseDocument(draft.value, { uniqueKeys: true, prettyErrors: false }), issue = doc.errors[0]
    if (issue) return { doc: null, config: {}, error: `YAML 第 ${draft.value.slice(0, issue.pos[0]).split('\n').length} 行附近存在语法错误。` }
    const raw: unknown = doc.toJS({ maxAliasCount: 40 })
    if (raw === null || typeof raw !== 'object' || Array.isArray(raw)) return { doc: null, config: {}, error: '配置根节点需要是 YAML 对象。' }
    return { doc: doc as Document, config: object(raw), error: '' }
  } catch { return { doc: null, config: {}, error: '无法解析 YAML，请检查格式或别名引用。' } }
})
function at(path: Path): unknown { let value: unknown = parsed.value.config; for (const key of path) { if (value === null || typeof value !== 'object') return undefined; value = (value as Record<string | number, unknown>)[key] } return value }
function listAt(path: Path): Obj[] { const value = at(path); return Array.isArray(value) ? value.map(object) : [] }
function fieldValue(path: Path, field: Field) { return at([...path, field.key]) ?? field.default }
function fieldId(path: Path, field: Field) { return 'config-' + [...path, field.key].join('-') }
function mutate(update: (doc: Document) => void) { const doc = parsed.value.doc?.clone(); if (!doc) return; try { update(doc); draft.value = doc.toString({ lineWidth: 0 }); error.value = ''; success.value = '' } catch { error.value = '配置字段格式无法编辑，请检查部署目录中的配置文件。' } }
function put(path: Path, value: unknown) { mutate(doc => { if (value === undefined) doc.deleteIn(path); else doc.setIn(path, value) }) }
function invalid(path: Path, field: Field, message: string) { invalidFields.value[[...path, field.key].join('.')] = message }
function append(path: Path, value: Obj) { mutate(doc => { if (!Array.isArray(at(path))) doc.setIn(path, [value]); else doc.addIn(path, value) }) }
function removeItem(path: Path, index: number) { for (const key of Object.keys(invalidFields.value)) if (key.startsWith(path.join('.') + '.')) delete invalidFields.value[key]; mutate(doc => { doc.deleteIn([...path, index]) }) }
const savedConfig = computed(() => { try { return object(parseDocument(savedYaml.value).toJS({ maxAliasCount: 40 })) } catch { return {} } })
const dirty = computed(() => !sameValue(parsed.value.config, savedConfig.value)), inputError = computed(() => Object.values(invalidFields.value).find(Boolean) ?? '')
const saveAreas: Record<string, Path[]> = {
  connection: [['ikuai']],
  webui: [['webui']],
  lines: [['wan', 'primary'], ['wan', 'backup'], ['wan', 'preserved_default'], ['wan', 'enabled'], ['management', 'apply']],
  rules: [['wan', 'rule_pairs']],
  timing: [['wan', 'check_interval_ms'], ['wan', 'failure_threshold'], ['wan', 'recovery_threshold'], ['wan', 'stable_hold_ms']],
  ipv6: [['wan', 'ipv6_enabled'], ['wan', 'ipv6']],
  channels: [['notifications', 'channels'], ['notifications', 'secrets']],
  delivery: [['notifications', 'enabled'], ['notifications', 'events'], ['notifications', 'timeout_ms'], ['notifications', 'cooldown_ms'], ['notifications', 'queue_capacity']],
  templates: [['notifications', 'templates']],
  management: [['management', 'listen'], ['management', 'state_dir'], ['management', 'web_dir']],
}
function valueAt(config: Obj, path: Path): unknown { let value: unknown = config; for (const key of path) value = object(value)[String(key)]; return value }
function areaChanged(area: string) { return (saveAreas[area] ?? []).some(path => !sameValue(at(path), valueAt(savedConfig.value, path))) }
function canSaveArea(area: string) {
  const paths = saveAreas[area] ?? []
  return !loading.value && !saving.value && Boolean(revision.value) && !parsed.value.error && areaChanged(area) && !Object.entries(invalidFields.value).some(([key, message]) => message && (key === 'credentials-conflict' || paths.some(path => key === path.join('.') || key.startsWith(path.join('.') + '.'))))
}
function categoryEnabled(category: typeof notificationTemplateCategories[number]) { return at(['notifications', 'enabled']) !== false && (!selectedEvents.value.length || category.events.some(kind => selectedEvents.value.includes(kind))) }
function categoryLabel(category: typeof notificationTemplateCategories[number]) { if (!categoryEnabled(category)) return '不发送'; const count = selectedEvents.value.length ? category.events.filter(kind => selectedEvents.value.includes(kind)).length : category.events.length; return count === category.events.length ? '发送此类事件' : `已选择 ${count}/${category.events.length} 个事件` }
function setCategory(category: typeof notificationTemplateCategories[number], enabled: boolean) {
  const current = at(['notifications', 'enabled']) === false ? [] : selectedEvents.value.length ? selectedEvents.value : notificationTypes.map(item => item.value)
  const values = enabled ? [...new Set([...current, ...category.events])] : current.filter(kind => !(category.events as readonly string[]).includes(kind))
  mutate(doc => { doc.setIn(['notifications', 'enabled'], values.length > 0); doc.setIn(['notifications', 'events'], values.length === notificationTypes.length ? [] : values) })
}
const rulePairs = computed(() => listAt(['wan', 'rule_pairs'])), channels = computed(() => listAt(['notifications', 'channels']))
const ipv6Configured = computed(() => Object.keys(object(at(['wan', 'ipv6']))).length > 0)
const selectedEvents = computed(() => Array.isArray(at(['notifications', 'events'])) ? at(['notifications', 'events']) as string[] : [])
const connection = computed(() => ({ base_url: String(at(['ikuai', 'base_url']) ?? ''), username: String(at(['ikuai', 'username']) ?? ''), password: String(at(['ikuai', 'password']) ?? ''), allow_invalid_certificate: at(['ikuai', 'allow_invalid_certificate']) === true, timeout_ms: Number(at(['ikuai', 'timeout_ms']) ?? 5000) }))
const connectionStamp = computed(() => JSON.stringify(connection.value))
const discoveryReady = computed(() => options.value !== null && discoveredConnection.value === connectionStamp.value)
const wanInterfaces = computed(() => options.value?.interfaces.filter(option => option.kind === 'wan') ?? [])
function interfaceLabel(name: unknown) { const value = String(name ?? ''); return options.value?.interfaces.find(option => option.name === value)?.label || value.toUpperCase() || '未选择' }
function duplicateInterface(key: string, value: string) { return ['primary', 'backup', 'preserved_default'].some(field => field !== key && at(['wan', field]) === value) }
function interfaceDisabled(key: string, option: Interface) { return duplicateInterface(key, option.name) || (key === 'preserved_default' ? !option.default_route : option.default_route) }
function resourceLabel(value: unknown) { return resources.find(resource => resource.value === value)?.label ?? '分流规则' }
function ruleBySide(index: number, side: Side) { const pair = rulePairs.value[index]; return options.value?.rules[String(pair?.resource) as Resource]?.find(rule => rule.id === pair?.[`${side}_id`] && rule.tag === pair?.[`${side}_tag`]) }
function rulesFor(index: number, side: Side) { const pair = rulePairs.value[index]; return options.value?.rules[String(pair?.resource) as Resource]?.filter(rule => rule.interface === at(['wan', side])) ?? [] }
function usedRule(index: number, rule: Rule) { return rulePairs.value.some((pair, other) => other !== index && pair.resource === rulePairs.value[index]?.resource && (pair.primary_id === rule.id || pair.backup_id === rule.id)) }
function ruleDisabled(index: number, side: Side, rule: Rule) { if (!rule.selectable || usedRule(index, rule)) return true; const primary = ruleBySide(index, 'primary'); return side === 'backup' && (!primary || !primary.selectable || !primary.selector_fingerprint || primary.selector_fingerprint !== rule.selector_fingerprint || primary.id === rule.id) }
function ruleLabel(index: number, side: Side, rule: Rule) { return rule.label + (!rule.selectable ? `（${rule.reason || '暂不可管理'}）` : usedRule(index, rule) ? '（已用于其他规则）' : side === 'backup' && ruleDisabled(index, side, rule) ? '（匹配条件不同）' : '') }
function ruleValue(index: number, side: Side) { const pair = rulePairs.value[index]; return pair?.[`${side}_id`] ? `${String(pair[`${side}_id`])}:${String(pair[`${side}_tag`] ?? '')}` : '' }
function interfaceItems(key: string) { return wanInterfaces.value.map(option => ({ value: option.name, title: option.label, props: { disabled: interfaceDisabled(key, option) } })) }
function ruleItems(index: number, side: Side) { return rulesFor(index, side).map(rule => ({ value: `${rule.id}:${rule.tag}`, title: ruleLabel(index, side, rule), props: { disabled: ruleDisabled(index, side, rule) } })) }
function ipv6Items(side: 'wan' | 'lan') { return (options.value?.ipv6[side] ?? []).map(option => ({ value: `${option.id}:${option.tag}`, title: option.label, props: { disabled: !option.selectable } })) }
function changeResource(index: number, value: unknown) { put(['wan', 'rule_pairs', index], { resource: String(value), primary_id: 0, backup_id: 0, primary_tag: '', backup_tag: '' }) }
function chooseRule(index: number, side: Side, selected: unknown) {
  const value = String(selected ?? ''), rule = rulesFor(index, side).find(option => `${option.id}:${option.tag}` === value)
  if (!rule || ruleDisabled(index, side, rule)) return
  mutate(doc => {
    doc.setIn(['wan', 'rule_pairs', index, `${side}_id`], rule.id); doc.setIn(['wan', 'rule_pairs', index, `${side}_tag`], rule.tag)
    if (side === 'primary') {
      const current = ruleBySide(index, 'backup')
      if (current && current.selectable && current.interface === at(['wan', 'backup']) && current.selector_fingerprint === rule.selector_fingerprint) return
      const matches = rulesFor(index, 'backup').filter(candidate => candidate.selectable && candidate.id !== rule.id && candidate.selector_fingerprint && candidate.selector_fingerprint === rule.selector_fingerprint && !usedRule(index, candidate))
      const backup = matches.length === 1 ? matches[0] : undefined
      doc.setIn(['wan', 'rule_pairs', index, 'backup_id'], backup?.id ?? 0); doc.setIn(['wan', 'rule_pairs', index, 'backup_tag'], backup?.tag ?? '')
    }
  })
}
function addPair(resource: Resource) { append(['wan', 'rule_pairs'], { resource, primary_id: 0, backup_id: 0, primary_tag: '', backup_tag: '' }) }
function pairIssue(index: number) { if (!discoveryReady.value) return ''; const primary = ruleBySide(index, 'primary'), backup = ruleBySide(index, 'backup'); if (!primary || !backup) return '请选择主线路与备用线路对应的规则。'; if (primary.interface !== at(['wan', 'primary']) || backup.interface !== at(['wan', 'backup'])) return '规则出口与当前主备线路不同，请重新选择。'; if (!primary.selectable || !backup.selectable) return '规则暂不符合管理条件。'; if (!primary.selector_fingerprint || primary.selector_fingerprint !== backup.selector_fingerprint) return '主备规则需要匹配相同流量。'; return '' }
function ipv6Value(side: 'wan' | 'lan') { const value = at(['wan', 'ipv6', `${side}_id`]); return value ? `${String(value)}:${String(at(['wan', 'ipv6', `${side}_tag`]) ?? '')}` : '' }
function chooseIpv6(side: 'wan' | 'lan', selected: unknown) {
  const value = String(selected ?? '')
  const option = options.value?.ipv6[side].find(item => `${item.id}:${item.tag}` === value)
  if (!option?.selectable) return
  mutate(doc => {
    const existing = doc.getIn(['wan', 'ipv6'], true)
    if (existing === undefined || existing === null || (isScalar(existing) && existing.value === null)) {
      // Nested writes in this transaction require a YAML map, not a plain object.
      const profile = doc.createNode({ settle_timeout_ms: 60000, poll_interval_ms: 1000 })
      if (isScalar(existing)) { profile.commentBefore = existing.commentBefore; profile.comment = existing.comment }
      doc.setIn(['wan', 'ipv6'], profile)
    } else if (!isMap(existing)) {
      throw new Error('IPv6 settings must be a YAML map')
    }
    if (!doc.hasIn(['wan', 'ipv6', 'settle_timeout_ms'])) doc.setIn(['wan', 'ipv6', 'settle_timeout_ms'], 60000)
    if (!doc.hasIn(['wan', 'ipv6', 'poll_interval_ms'])) doc.setIn(['wan', 'ipv6', 'poll_interval_ms'], 1000)
    doc.setIn(['wan', 'ipv6', `${side}_id`], option.id)
    doc.setIn(['wan', 'ipv6', `${side}_tag`], option.tag)
    if (side === 'lan') doc.setIn(['wan', 'ipv6', 'lan_interface'], option.interface)
  })
}
function failure(reason: unknown, connecting = false) { const data = object(reason), payload = object(data.payload ?? data.body); if (data.status === 401) return '登录已过期，请重新登录。'; if (data.status === 409) return '配置已被其他方式修改，请刷新页面后再保存。'; if (connecting) return typeof payload.message === 'string' ? payload.message : '无法读取爱快选项，请检查管理地址、账号密码和证书设置。'; if (data.status === 400) return typeof payload.line === 'number' ? `配置第 ${payload.line} 行附近未通过校验。` : '请检查连接信息、主备规则和通知设置。'; if (data.status === 413) return '配置文件过大。'; return '配置读取或保存失败，请确认服务可连接且配置目录可写。' }
async function discover() { if (discovering.value || parsed.value.error) return; const version = ++discoveryGeneration, stamp = connectionStamp.value; discovering.value = true; discoveryError.value = ''; try { const result = object(await props.request('/settings/options', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(connection.value) })); if (!mounted || version !== discoveryGeneration || stamp !== connectionStamp.value) return; if (!Array.isArray(result.interfaces) || !result.rules || !result.ipv6) throw new Error(); options.value = result as unknown as Options; discoveredConnection.value = stamp } catch (reason: unknown) { if (mounted && version === discoveryGeneration) discoveryError.value = failure(reason, true) } finally { if (mounted && version === discoveryGeneration) discovering.value = false } }
async function load() { if (loading.value || saving.value) return; const version = ++generation; loading.value = true; error.value = ''; success.value = ''; try { const result = object(await props.request('/settings/config')); if (!mounted || version !== generation) return; if (typeof result.yaml !== 'string' || typeof result.revision !== 'string') throw new Error(); draft.value = result.yaml; savedYaml.value = result.yaml; revision.value = result.revision; invalidFields.value = {}; options.value = null; discoveryError.value = ''; if (!parsed.value.error) { const stamp = connectionStamp.value; discovering.value = true; try { const result = object(await props.request('/settings/options')); if (mounted && version === generation && stamp === connectionStamp.value && Array.isArray(result.interfaces) && result.rules && result.ipv6) { options.value = result as unknown as Options; discoveredConnection.value = stamp } } catch { /* First setup stays usable before the router account is filled. */ } finally { if (mounted && version === generation) discovering.value = false } } } catch (reason: unknown) { if (mounted && version === generation) error.value = failure(reason) } finally { if (mounted && version === generation) loading.value = false } }
function canonicalValue(value: unknown): unknown { if (Array.isArray(value)) return value.map(canonicalValue); if (value !== null && typeof value === 'object') return Object.fromEntries(Object.entries(object(value)).sort(([left], [right]) => left.localeCompare(right)).map(([key, item]) => [key, canonicalValue(item)])); return value }
function sameValue(left: unknown, right: unknown) { return JSON.stringify(canonicalValue(left)) === JSON.stringify(canonicalValue(right)) }
function mergeValue(base: unknown, local: unknown, latest: unknown, path: string, conflicts: string[]): unknown {
  if (sameValue(latest, base) || sameValue(local, latest)) return local
  if (sameValue(local, base)) return latest
  const maps = [base, local, latest].every(value => value !== null && typeof value === 'object' && !Array.isArray(value))
  if (maps) {
    const original = object(base), current = object(local), remote = object(latest), result: Obj = {}
    for (const key of new Set([...Object.keys(original), ...Object.keys(current), ...Object.keys(remote)])) {
      const value = mergeValue(original[key], current[key], remote[key], path ? `${path}.${key}` : key, conflicts)
      if (value !== undefined) result[key] = value
    }
    return result
  }
  if (Array.isArray(base) && Array.isArray(local) && Array.isArray(latest) && path === 'notifications.channels') {
    const byTag = (items: unknown[]) => new Map(items.map(item => [String(object(item).tag), item]))
    const original = byTag(base), current = byTag(local), remote = byTag(latest)
    const order = [...current.keys(), ...[...remote.keys()].filter(key => !current.has(key))]
    return order.map(tag => mergeValue(original.get(tag), current.get(tag), remote.get(tag), `${path}.${tag}`, conflicts)).filter(value => value !== undefined)
  }
  conflicts.push(path)
  return local
}
function applyMerged(doc: Document, current: unknown, merged: unknown, path: Path = []) {
  if (sameValue(current, merged)) return
  if (current !== null && merged !== null && typeof current === 'object' && typeof merged === 'object' && !Array.isArray(current) && !Array.isArray(merged)) {
    const before = object(current), after = object(merged)
    for (const key of new Set([...Object.keys(before), ...Object.keys(after)])) applyMerged(doc, before[key], after[key], [...path, key])
  } else if (merged === undefined) doc.deleteIn(path)
  else doc.setIn(path, doc.createNode(merged))
}
async function refreshCredentials() {
  if (loading.value || saving.value) return
  const version = ++generation
  loading.value = true
  try {
    const result = object(await props.request('/settings/config'))
    if (!mounted || version !== generation) return
    if (typeof result.yaml !== 'string' || typeof result.revision !== 'string') throw new Error()
    const latest = parseDocument(result.yaml, { uniqueKeys: true, prettyErrors: false })
    if (latest.errors.length) throw new Error()
    if (!parsed.value.doc || parsed.value.error) {
      error.value = '通知凭据已更新，当前未保存内容已保留。请修正配置后再保存。'
      return
    }
    const original = parseDocument(savedYaml.value, { uniqueKeys: true, prettyErrors: false })
    const current = parsed.value.config, remote: unknown = latest.toJS({ maxAliasCount: 40 }), conflicts: string[] = []
    const merged = mergeValue(original.toJS({ maxAliasCount: 40 }), current, remote, '', conflicts)
    const document = parsed.value.doc.clone(), connectionBefore = connectionStamp.value
    applyMerged(document, current, merged)
    draft.value = document.toString({ lineWidth: 0 })
    savedYaml.value = result.yaml
    revision.value = result.revision
    if (connectionBefore !== connectionStamp.value) { options.value = null; discoveredConnection.value = '' }
    if (conflicts.length) {
      invalidFields.value['credentials-conflict'] = `配置中的 ${conflicts.join('、')} 已被其他方式更新。未保存内容已保留，请刷新页面重新读取后再保存。`
    } else {
      success.value = '通知凭据已更新，未保存的输入已保留。'
      if (!inputError.value) restartDialog.value = true
    }
  } catch { if (mounted && version === generation) error.value = '通知凭据已更新，但暂时无法读取。当前输入已保留，请稍后再试。' }
  finally { if (mounted && version === generation) loading.value = false }
}
async function saveArea(area: string) {
  if (!canSaveArea(area)) return
  const document = parseDocument(savedYaml.value, { uniqueKeys: true, prettyErrors: false })
  if (document.errors.length) return
  for (const path of saveAreas[area] ?? []) {
    for (let length = 1; length < path.length; length++) {
      const prefix = path.slice(0, length), node = document.getIn(prefix, true)
      if (node === undefined || node === null || (isScalar(node) && node.value === null)) document.setIn(prefix, document.createNode({}))
    }
    const value = at(path)
    if (value === undefined) document.deleteIn(path)
    else document.setIn(path, document.createNode(value))
  }
  const yaml = document.toString({ lineWidth: 0 })
  saving.value = true; savingArea.value = area; error.value = ''; success.value = ''
  try {
    const result = object(await props.request('/settings/config', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ yaml, revision: revision.value }) }))
    if (!mounted) return
    if (result.success !== true || typeof result.revision !== 'string') throw new Error()
    savedYaml.value = yaml; revision.value = result.revision; success.value = '当前卡片的修改已保存。'; emit('saved')
    if (result.restart_required === true) { await nextTick(); restartDialog.value = true }
  } catch (reason: unknown) { if (mounted) error.value = failure(reason) }
  finally { if (mounted) { saving.value = false; savingArea.value = '' } }
}
function restart() { restartDialog.value = false; emit('restart') }
function warnUnsaved(event: BeforeUnloadEvent) { if (dirty.value) { event.preventDefault(); event.returnValue = '' } }
onMounted(() => { window.addEventListener('beforeunload', warnUnsaved); void load() })
onUnmounted(() => { mounted = false; generation += 1; discoveryGeneration += 1; window.removeEventListener('beforeunload', warnUnsaved) })
</script>

<template>
  <section class="config-editor" :class="{ embedded }">
    <VAlert v-if="error || parsed.error || inputError" type="error" variant="tonal" class="mb-4">{{ error || parsed.error || inputError }}</VAlert><VAlert v-if="success" type="success" variant="tonal" class="mb-4">{{ success }}</VAlert>
    <VProgressLinear v-if="loading && !draft" indeterminate color="primary" />
    <template v-else-if="parsed.doc">
      <VTabs v-if="!embedded" v-model="group" color="primary" class="settings-tabs mb-5" show-arrows><VTab v-for="item in groups" :key="item.id" :value="item.id"><VIcon :icon="item.icon" size="18" class="mr-2" />{{ item.label }}</VTab></VTabs>
      <VForm class="settings-form" :disabled="loading || saving" @submit.prevent><VWindow v-model="group" :touch="false" class="settings-content-window">
        <VWindowItem value="connection"><VRow><VCol cols="12">
          <VCard class="settings-card"><VCardItem title="连接爱快" subtitle="填写爱快管理页面的账号密码。"><template #append><VChip :color="discoveryReady ? 'success' : 'default'" size="small">{{ discoveryReady ? '已连接' : '待连接' }}</VChip></template></VCardItem><VCardText><VRow><VCol v-for="field in [...fields.ikuai!, ...fields.connectionAdvanced!]" :key="field.key" cols="12" md="6"><ConfigField :field="field" :id="fieldId(['ikuai'], field)" :value="fieldValue(['ikuai'], field)" @update="put(['ikuai', field.key], $event)" @invalid="invalid(['ikuai'], field, $event)" /></VCol></VRow><VAlert v-if="discoveryError" type="error" variant="tonal" class="mt-4">{{ discoveryError }}</VAlert></VCardText><VCardActions class="px-5 pb-5"><VBtn :loading="savingArea === 'connection'" :disabled="!canSaveArea('connection')" prepend-icon="mdi-content-save" @click="saveArea('connection')">保存</VBtn><VBtn color="primary" :loading="discovering" prepend-icon="mdi-router-wireless" @click="discover">{{ discoveryReady ? '刷新接口和规则' : '连接并读取接口和规则' }}</VBtn></VCardActions></VCard>
          <VCard class="settings-card"><VCardItem title="网页登录" subtitle="用于登录本服务，与爱快账号分开设置。" /><VCardText><VRow><VCol v-for="field in [...fields.webui!, ...fields.loginTiming!]" :key="field.key" cols="12" md="6"><ConfigField :field="field" :id="fieldId(['webui'], field)" :value="fieldValue(['webui'], field)" @update="put(['webui', field.key], $event)" @invalid="invalid(['webui'], field, $event)" /></VCol></VRow></VCardText><VCardActions class="px-5 pb-5"><VBtn :loading="savingArea === 'webui'" :disabled="!canSaveArea('webui')" prepend-icon="mdi-content-save" @click="saveArea('webui')">保存</VBtn></VCardActions></VCard>
        </VCol></VRow></VWindowItem>
        <VWindowItem value="wan"><VRow><VCol cols="12">
          <VAlert v-if="!discoveryReady" type="info" variant="tonal" class="mb-4">先从爱快读取接口和规则。<VBtn variant="text" :loading="discovering" @click="discover">读取选项</VBtn></VAlert>
          <VCard class="settings-card"><VCardItem title="出口线路" subtitle="故障时使用备用线路，恢复后切回主线路。" /><VCardText><VRow><VCol v-for="item in [{ key: 'primary', label: '主线路' }, { key: 'backup', label: '备用线路' }, { key: 'preserved_default', label: '旁路由出口' }]" :key="item.key" cols="12" md="4"><VSelect prepend-inner-icon="mdi-router-wireless" :label="item.label" :model-value="String(at(['wan', item.key]) ?? '')" :items="interfaceItems(item.key)" :disabled="!discoveryReady" @update:model-value="put(['wan', item.key], $event)" /></VCol></VRow><VRow><VCol cols="12" md="6"><ConfigField :field="{ key: 'enabled', label: '检测线路状态', kind: 'boolean', default: true }" id="wan-enabled" :value="at(['wan', 'enabled'])" @update="put(['wan', 'enabled'], $event)" /></VCol><VCol cols="12" md="6"><ConfigField :field="{ key: 'apply', label: '启用故障转移', kind: 'boolean', hint: '开启后会按所选规则自动切换。', default: false }" id="auto-apply" :value="at(['management', 'apply'])" @update="put(['management', 'apply'], $event)" /></VCol></VRow></VCardText><VCardActions class="px-5 pb-5"><VBtn :loading="savingArea === 'lines'" :disabled="!canSaveArea('lines')" prepend-icon="mdi-content-save" @click="saveArea('lines')">保存</VBtn></VCardActions></VCard>
          <VCard class="settings-card"><VCardItem title="跟随切换的规则" subtitle="选择爱快中主备线路对应的规则。"><template #append><VChip size="small">{{ rulePairs.length }} 组</VChip></template></VCardItem><VCardText><VAlert v-if="!rulePairs.length" type="info" variant="tonal" class="mt-4">添加需要跟随出口切换的防回环、运营商分流或域名分流规则。</VAlert><VCard v-for="(pair, index) in rulePairs" :key="index" class="rule-card" variant="outlined"><VCardItem :title="resourceLabel(pair.resource)"><template #append><VBtn icon="mdi-delete-outline" size="small" variant="text" color="error" aria-label="移除规则" @click="removeItem(['wan', 'rule_pairs'], index)" /></template></VCardItem><VCardText><VSelect prepend-inner-icon="mdi-router-wireless" :model-value="String(pair.resource)" label="规则类型" class="rule-type-input" :items="resources" item-title="label" item-value="value" @update:model-value="changeResource(index, $event)" /><VRow><VCol v-for="side in ['primary', 'backup'] as const" :key="side" cols="12" md="6"><VSelect prepend-inner-icon="mdi-router-wireless" :model-value="ruleValue(index, side)" :label="side === 'primary' ? '主线路规则' : '备用线路规则'" :hint="interfaceLabel(at(['wan', side]))" persistent-hint :items="ruleItems(index, side)" :disabled="!discoveryReady || (side === 'backup' && !ruleBySide(index, 'primary'))" @update:model-value="chooseRule(index, side, $event)" /></VCol></VRow><VAlert v-if="pairIssue(index)" type="warning" variant="tonal" class="mt-3">{{ pairIssue(index) }}</VAlert></VCardText></VCard></VCardText><VCardActions class="px-5 pb-5"><VBtn :loading="savingArea === 'rules'" :disabled="!canSaveArea('rules')" prepend-icon="mdi-content-save" @click="saveArea('rules')">保存</VBtn><VMenu location="bottom start"><template #activator="{ props: menuProps }"><VBtn v-bind="menuProps" color="success" variant="tonal" min-width="48" :disabled="!discoveryReady" aria-label="添加规则"><VIcon icon="mdi-plus" /></VBtn></template><VList><VListItem v-for="resource in resources" :key="resource.value" :title="resource.label" @click="addPair(resource.value)" /></VList></VMenu></VCardActions></VCard>
          <VCard class="settings-card"><VCardItem title="检测与回切时间" /><VCardText><VRow><VCol v-for="field in fields.timing" :key="field.key" cols="12" md="6"><ConfigField :field="field" :id="fieldId(['wan'], field)" :value="fieldValue(['wan'], field)" @update="put(['wan', field.key], $event)" @invalid="invalid(['wan'], field, $event)" /></VCol></VRow></VCardText><VCardActions class="px-5 pb-5"><VBtn :loading="savingArea === 'timing'" :disabled="!canSaveArea('timing')" prepend-icon="mdi-content-save" @click="saveArea('timing')">保存</VBtn></VCardActions></VCard>
        </VCol></VRow></VWindowItem>
        <VWindowItem value="ipv6"><VRow><VCol cols="12">
          <VCard class="settings-card"><VCardItem title="IPv6 出口与内网" subtitle="选择爱快中已有的 IPv6 配置。" /><VCardText><ConfigField :field="{ key: 'ipv6_enabled', label: '启用 IPv6 故障转移', kind: 'boolean', hint: '切换线路时同步更新 IPv6 出口。', default: false }" id="ipv6-enabled" :value="at(['wan', 'ipv6_enabled'])" @update="put(['wan', 'ipv6_enabled'], $event)" /><VAlert v-if="!discoveryReady" type="info" variant="tonal" class="my-4">需要先从爱快读取 IPv6 配置。<VBtn variant="text" :loading="discovering" @click="discover">读取选项</VBtn></VAlert><VRow class="mt-5"><VCol v-for="side in ['wan', 'lan'] as const" :key="side" cols="12" md="6"><VSelect prepend-inner-icon="mdi-router-wireless" :model-value="ipv6Value(side)" :label="side === 'wan' ? 'IPv6 出口' : 'IPv6 内网'" :items="ipv6Items(side)" :disabled="!discoveryReady" @update:model-value="chooseIpv6(side, $event)" /></VCol><VCol cols="12" md="6"><VTextField prepend-inner-icon="mdi-lan" label="内网接口" :model-value="interfaceLabel(at(['wan', 'ipv6', 'lan_interface']))" readonly hint="随内网配置自动读取。" persistent-hint /></VCol></VRow><VRow v-if="ipv6Configured" class="mt-4"><VCol v-for="field in fields.ipv6Timing" :key="field.key" cols="12" md="6"><ConfigField :field="field" :id="fieldId(['wan', 'ipv6'], field)" :value="fieldValue(['wan', 'ipv6'], field)" @update="put(['wan', 'ipv6', field.key], $event)" @invalid="invalid(['wan', 'ipv6'], field, $event)" /></VCol></VRow><VAlert v-for="warning in options?.warnings.filter(item => item.section === 'ipv6') ?? []" :key="warning.message" type="warning" variant="tonal" class="mt-4">{{ warning.message }}</VAlert></VCardText><VCardActions class="px-5 pb-5"><VBtn :loading="savingArea === 'ipv6'" :disabled="!canSaveArea('ipv6')" prepend-icon="mdi-content-save" @click="saveArea('ipv6')">保存</VBtn></VCardActions></VCard>
        </VCol></VRow></VWindowItem>
        <VWindowItem value="notifications"><VRow><VCol cols="12">
          <NotificationChannelSettings :model-value="channels" :secrets="object(at(['notifications', 'secrets']))" :notifications="object(at(['notifications']))" :request="request" :disabled="loading || saving" :credentials-enabled="!dirty && !inputError" @refresh="refreshCredentials" @update:model-value="put(['notifications', 'channels'], $event)" @secret="(key, value) => put(['notifications', 'secrets', key], value)" @invalid="(key, message) => invalidFields[`notifications.${key}`] = message"><template #actions><VBtn :loading="savingArea === 'channels'" :disabled="!canSaveArea('channels')" prepend-icon="mdi-content-save" @click="saveArea('channels')">保存</VBtn></template></NotificationChannelSettings>
          <NotificationTemplateEditor class="mb-6" :notifications="object(at(['notifications']))" :model-value="notificationTemplates" :events="templateEvents" :request="request" @update:model-value="put(['notifications', 'templates'], $event)" @save="saveArea('templates')" @invalid="message => invalidFields['notifications.templates'] = message" />
          <VCard class="settings-card"><VCardItem title="通知发送范围" subtitle="选择需要发送的事件类型，具体接收目标在各通知渠道中设置。" /><VCardText><ConfigField :field="notificationEnabledField" id="notify-enabled" :value="at(['notifications', 'enabled'])" @update="put(['notifications', 'enabled'], $event)" /></VCardText><VTable class="notification-scope-table"><thead><tr><th>消息类型</th><th>范围</th></tr></thead><tbody><tr v-for="category in notificationTemplateCategories" :key="category.kind"><td><VIcon :icon="category.icon" size="18" class="mr-2" />{{ category.label }}</td><td><VCheckbox :model-value="categoryEnabled(category)" :label="categoryLabel(category)" hide-details density="compact" @update:model-value="setCategory(category, Boolean($event))" /></td></tr></tbody></VTable><VCardText><VRow><VCol v-for="field in fields.notificationTiming" :key="field.key" cols="12" md="4"><ConfigField :field="field" :id="fieldId(['notifications'], field)" :value="fieldValue(['notifications'], field)" @update="put(['notifications', field.key], $event)" @invalid="invalid(['notifications'], field, $event)" /></VCol></VRow></VCardText><VCardActions class="px-5 pb-5"><VBtn :loading="savingArea === 'delivery'" :disabled="!canSaveArea('delivery')" prepend-icon="mdi-content-save" @click="saveArea('delivery')">保存</VBtn></VCardActions></VCard>
        </VCol></VRow></VWindowItem>
        <VWindowItem value="management"><VRow><VCol cols="12"><VCard class="settings-card"><VCardItem title="服务设置" /><VCardText><VRow><VCol v-for="field in fields.management" :key="field.key" cols="12" md="6"><ConfigField :field="field" :id="fieldId(['management'], field)" :value="fieldValue(['management'], field)" @update="put(['management', field.key], $event)" @invalid="invalid(['management'], field, $event)" /></VCol></VRow></VCardText><VCardActions class="px-5 pb-5"><VBtn :loading="savingArea === 'management'" :disabled="!canSaveArea('management')" prepend-icon="mdi-content-save" @click="saveArea('management')">保存</VBtn></VCardActions></VCard></VCol></VRow></VWindowItem>
        <VAlert v-if="discoveryError && group !== 'connection'" type="error" variant="tonal" class="mb-4">{{ discoveryError }}</VAlert>
      </VWindow></VForm>
    </template>

    <VDialog v-model="restartDialog" max-width="450"><VCard><VCardItem title="配置已保存" /><VCardText>重启服务后使用新配置。</VCardText><VCardActions><VSpacer /><VBtn variant="text" @click="restartDialog = false">稍后重启</VBtn><VBtn color="primary" @click="restart">现在重启</VBtn></VCardActions></VCard></VDialog>

  </section>
</template>
<style scoped>
.config-editor{min-width:0;width:100%}.settings-tabs{margin-bottom:20px}.settings-card{margin-bottom:24px}.settings-content-window{overflow:hidden;padding:0 12px;margin:0 -12px}.settings-form{min-inline-size:0}.rule-card{margin-top:16px}.rule-type-input{margin-bottom:28px}.rule-type-input+.v-row{margin-top:0}.notification-scope-table td:first-child{width:30%;white-space:nowrap}.notification-scope-table .v-checkbox{margin-block:6px}@media(max-width:600px){.notification-scope-table td:first-child{width:45%}.notification-scope-table td{padding-inline:12px}}
</style>
