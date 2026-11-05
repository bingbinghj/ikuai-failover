export type Health = 'up' | 'down' | 'unknown'
export type Egress = 'primary' | 'backup'
export interface Session { username: string }
export interface IkuaiSettings {
  revision: string
  base_url: string
  username: string
  password_configured: boolean
  allow_invalid_certificate: boolean
  timeout_ms: number
  environment_overrides: { username: boolean; password: boolean }
}
export interface IkuaiSettingsUpdate {
  revision: string
  base_url: string
  username: string
  password?: string
  clear_password: boolean
  allow_invalid_certificate: boolean
  timeout_ms: number
}
export interface Line { name: string; address: string; health: Health }
export interface Ipv6Line { name: string; address: string; prefix: string }
export interface Delivery { tag: string; provider: string; success: boolean; errorCode: string }
export interface Notice {
  id: string
  kind: string
  title: string
  text: string
  timestamp: number
  deliveries: Delivery[]
  pending: string[]
  status: string
}
export interface PolicyChange { resource: string; id: number; tag: string; field: string; before: string; after: string }
export interface IkuaiInfo {
  hostname?: string
  version?: string
  model?: string
  uptimeSeconds?: number
  cpuPercent?: number
  memoryPercent?: number
  memoryTotalKib?: number
  stale?: boolean
  updatedAtMs?: number
  usageSampledAt?: number
}
export interface Snapshot {
  ikuaiInfo: IkuaiInfo | null
  active: Egress
  primaryName: string
  backupName: string
  preservedDefault: string
  primaryHealth: Health
  backupHealth: Health
  applyEnabled: boolean
  wanEnabled: boolean
  setupRequired: boolean
  interfaceNames: Record<string, string>
  ipv6Capability: string
  ipv6Ready: { address: boolean; prefix: boolean; lan: boolean } | null
  ipv6Error: string
  lastError: string
  blockers: string[]
  changes: PolicyChange[]
  target: Egress
  lines: Line[]
  ipv6: Ipv6Line[]
  lastTransition: number
  notifications: { enabled: boolean; queued: number; history: number; delivered: number; failed: number }
}

export function object(value: unknown): Record<string, unknown> {
  return value !== null && typeof value === 'object' && !Array.isArray(value) ? value as Record<string, unknown> : {}
}
const string = (value: unknown, fallback = '') => typeof value === 'string' ? value : fallback
const number = (value: unknown) => typeof value === 'number' && Number.isFinite(value) ? value : 0
const array = (value: unknown): unknown[] => Array.isArray(value) ? value : []
const strings = (value: unknown) => array(value).filter((item): item is string => typeof item === 'string')
const health = (value: unknown): Health => value === 'up' || value === 'down' ? value : 'unknown'
const egress = (value: unknown): Egress => value === 'backup' ? 'backup' : 'primary'

function parseIkuaiInfo(value: unknown): IkuaiInfo | null {
  const raw = object(value), info: IkuaiInfo = {}
  for (const key of ['hostname', 'version', 'model'] as const) {
    const value = raw[key]
    if (typeof value === 'string' && value.trim()) info[key] = value.trim()
  }
  for (const [source, target] of [['uptime_seconds', 'uptimeSeconds'], ['cpu_percent', 'cpuPercent'], ['memory_percent', 'memoryPercent'], ['memory_total_kib', 'memoryTotalKib'], ['updated_at_ms', 'updatedAtMs'], ['usage_sampled_at', 'usageSampledAt']] as const) {
    const value = raw[source]
    if (typeof value === 'number' && Number.isFinite(value) && value >= 0) info[target] = value
  }
  if (typeof raw.stale === 'boolean') info.stale = raw.stale
  return Object.keys(info).length ? info : null
}

export function parseSnapshot(value: unknown): Snapshot {
  const root = object(value)
  if (!root.wan || typeof root.wan !== 'object') throw new Error('状态数据暂不可用')
  const wan = object(root.wan)
  const state = object(wan.state)
  if (state.active !== 'primary' && state.active !== 'backup') throw new Error('当前出口状态暂不可用')
  const plan = object(wan.plan)
  const addresses = object(wan.interface_addresses)
  const statuses = object(wan.interface_health ?? root.wan_health)
  const primaryName = string(root.primary_interface ?? wan.primary_interface, 'wan1')
  const backupName = string(root.backup_interface ?? wan.backup_interface, 'wan3')
  const preservedDefault = string(root.preserved_default, 'wan2')
  const known = new Set([primaryName, preservedDefault, backupName, ...Object.keys(addresses), ...Object.keys(statuses)])
  for (const item of array(root.interfaces)) {
    const line = object(item)
    const name = string(line.interface)
    if (name) { known.add(name); if (!addresses[name]) addresses[name] = string(line.ip_addr) }
  }
  const notification = object(root.notifications)
  const ikuaiInfo = parseIkuaiInfo(root.ikuai_info)
  const counters = object(notification.counters)
  const evidence = plan.ipv6_evidence ? object(plan.ipv6_evidence) : null
  return {
    ikuaiInfo,
    active: egress(state.active), primaryName, backupName, preservedDefault,
    primaryHealth: health(wan.primary), backupHealth: health(wan.backup),
    applyEnabled: wan.apply_enabled === true, wanEnabled: root.wan_enabled !== false,
    setupRequired: root.setup_required === true,
    interfaceNames: Object.fromEntries(Object.entries(object(root.interface_names)).filter((entry): entry is [string, string] => typeof entry[1] === 'string')),
    ipv6Capability: string(wan.ipv6_capability, 'checking'),
    ipv6Ready: evidence ? { address: evidence.wan_address_ready === true, prefix: evidence.delegated_prefix_ready === true, lan: evidence.lan_prefix_ready === true } : null,
    ipv6Error: string(wan.ipv6_error || plan.ipv6_error), lastError: string(wan.last_error),
    blockers: strings(plan.blockers), target: egress(plan.target),
    changes: array(plan.changes).map(item => {
      const change = object(item), before = object(change.before), after = object(change.after)
      const field = ['interface', 'parent', 'enabled'].find(key => before[key] !== after[key]) ?? 'enabled'
      return { resource: string(change.resource), id: number(change.id), tag: string(after.tagname ?? before.tagname), field, before: string(before[field]), after: string(after[field]) }
    }),
    lines: [...known].filter(Boolean).map(name => ({ name, address: string(addresses[name]), health: health(statuses[name] ?? (name === primaryName ? wan.primary : name === backupName ? wan.backup : undefined)) })),
    ipv6: array(root.ipv6_addresses).map(item => { const line = object(item); return { name: string(line.interface), address: string(line.dhcp6_ip_addr), prefix: string(line.dhcp6_prefix1) } }),
    lastTransition: number(state.last_transition_ms),
    notifications: { enabled: notification.enabled === true, queued: number(notification.queued_events), history: number(notification.history_events), delivered: number(counters.delivered ?? counters.sent), failed: number(counters.failed_attempts ?? counters.failed) },
  }
}

export function parseNotices(value: unknown): Notice[] {
  return array(object(value).events).map(item => {
    const row = object(item), event = object(row.event)
    return { id: string(event.id), kind: string(event.kind), title: string(event.title, '运行事件'), text: string(event.text), timestamp: number(event.timestamp_ms),
      deliveries: array(row.deliveries).map(item => { const delivery = object(item); return { tag: string(delivery.tag), provider: string(delivery.provider), success: delivery.success === true, errorCode: string(delivery.error_code) } }),
      pending: strings(row.pending_tags), status: string(row.status),
    }
  }).sort((left, right) => right.timestamp - left.timestamp)
}

export const eventLabels: Record<string, string> = {
  service_started: '服务启动', service_stopped: '服务停止', service_failed: '服务异常',
  configuration_saved: '配置已保存',
  service_restarting: '服务重启',
  wan_switched: '出口切换', wan_recovered: '主线恢复', wan_down: '线路中断', wan_up: '线路恢复',
  wan_health_changed: '线路健康变化', interface_health_changed: '线路健康变化',
  interface_address_changed: '接口地址变化', public_ip_changed: '公网地址变化',
  ipv6_address_changed: 'IPv6 地址变化', ipv6_prefix_changed: 'IPv6 前缀变化',
  ipv6_degraded: 'IPv6 异常', ipv6_recovered: 'IPv6 恢复', strategy_failed: '策略应用失败',
  strategy_applied: '策略已应用', policy_failed: '策略应用失败', policy_applied: '策略已应用',
  api_unreachable: '爱快连接异常', api_recovered: '爱快连接恢复',
  egress_switched: '出口切换', egress_recovered: '主线恢复回切', failover_pending: '等待故障转移',
  egress_selection_changed: '手动选择线路',
  wan_ipv4_changed: 'IPv4 地址变化', wan_ipv6_changed: 'IPv6 地址变化', ipv6_pd_changed: 'PD 前缀变化',
  policy_conflict: '策略冲突', cycle_failed: '线路检查失败', cycle_recovered: '线路检查恢复', policy_synchronized: '策略已同步',
}
export const providerLabels: Record<string, string> = {
  telegram: 'Telegram', slack: 'Slack', discord: 'Discord', ding_talk: '钉钉',
  synology_chat: 'Synology Chat', voce_chat: 'VoceChat', wechat: '企业微信', feishu: '飞书',
  qy_wechat_bot: '企业微信机器人',
  wechat_claw_bot: '微信 ClawBot', qq: 'QQ', web_push: '浏览器推送', app_push: 'AppPush', web: 'Web', web_agent: 'WebAgent',
}
export const errorLabels: Record<string, string> = {
  secret_missing: '缺少渠道凭据', network_error: '网络连接失败', http_rejected: '服务拒绝请求',
  provider_rejected: '渠道拒绝投递', invalid_response: '渠道响应异常', timeout: '请求超时',
  crypto_error: '推送密钥无效', invalid_configuration: '渠道配置有误',
}
export const healthLabel = (value: Health) => value === 'up' ? '正常' : value === 'down' ? '异常' : '检测中'
export const eventLabel = (kind: string) => eventLabels[kind] ?? '运行状态变化'
export const providerLabel = (name: string) => providerLabels[name] ?? name
export const timeLabel = (timestamp: number) => timestamp > 0 ? new Intl.DateTimeFormat('zh-CN', { month: '2-digit', day: '2-digit', hour: '2-digit', minute: '2-digit', second: '2-digit', hour12: false }).format(new Date(timestamp)) : '尚无记录'
export const isPrivateAddress = (address: string) => /^(10\.|192\.168\.|172\.(1[6-9]|2\d|3[01])\.|100\.(6[4-9]|[7-9]\d|1[01]\d|12[0-7])\.)/.test(address)
