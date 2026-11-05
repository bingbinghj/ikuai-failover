export const notificationEventGroups = [
  { title: '线路与地址', items: [['wan_health_changed', '线路健康状态'], ['wan_ipv4_changed', 'IPv4 地址变化'], ['wan_ipv6_changed', 'IPv6 地址变化'], ['ipv6_pd_changed', 'PD 前缀变化']] },
  { title: '切换与恢复', items: [['egress_switched', '出口切换'], ['egress_recovered', '恢复回切'], ['egress_selection_changed', '手动选择线路'], ['failover_pending', '等待故障转移'], ['ipv6_degraded', 'IPv6 异常'], ['ipv6_recovered', 'IPv6 恢复']] },
  { title: '策略与配置', items: [['policy_synchronized', '规则已同步'], ['policy_conflict', '规则冲突'], ['cycle_failed', '检查失败'], ['cycle_recovered', '检查恢复'], ['configuration_saved', '配置已保存']] },
] as const

export const notificationTypes = notificationEventGroups.flatMap(group => group.items.map(([value, title]) => ({ value, title })))
export const notificationTemplateEvents = notificationTypes.map(item => ({ kind: item.value, label: item.title }))
export const notificationTemplateCategories = [
  { kind: 'network_change', label: '线路与地址', icon: 'mdi-ip-network', color: 'primary', sampleKind: 'wan_ipv4_changed', events: ['wan_health_changed', 'wan_ipv4_changed', 'wan_ipv6_changed', 'ipv6_pd_changed'] },
  { kind: 'egress_change', label: '切换与恢复', icon: 'mdi-swap-horizontal', color: 'info', sampleKind: 'egress_switched', events: ['egress_switched', 'egress_recovered', 'egress_selection_changed', 'failover_pending', 'ipv6_degraded', 'ipv6_recovered'] },
  { kind: 'policy_change', label: '策略与配置', icon: 'mdi-filter-variant', color: 'warning', sampleKind: 'policy_synchronized', events: ['policy_synchronized', 'policy_conflict', 'cycle_failed', 'cycle_recovered', 'configuration_saved'] },
] as const
const knownKinds = new Set<string>(notificationTypes.map(item => item.value))
export function normalizeNotificationTypes(value: unknown): string[] {
  return Array.isArray(value) ? [...new Set(value.filter((item): item is string => typeof item === 'string' && knownKinds.has(item)))] : []
}

export const notificationSamples: Record<string, { title: string; text: string }> = {
  wan_health_changed: { title: '主线路状态变化', text: '主线路已离线，正在等待故障转移。' },
  wan_ipv4_changed: { title: 'IPv4 地址已更新', text: '主线路 IPv4 地址从 203.0.113.10 变为 203.0.113.20。' },
  wan_ipv6_changed: { title: 'IPv6 地址已更新', text: '主线路 IPv6 地址已更新为 2001:db8::20。' },
  ipv6_pd_changed: { title: 'PD 前缀已更新', text: '内网 PD 前缀已更新为 2001:db8:1200::/56。' },
  egress_switched: { title: '已切换到备用线路', text: '主线路连续检查失败，出口已切换到备用线路。' },
  egress_recovered: { title: '已恢复主线路', text: '主线路恢复稳定，出口已切回主线路。' },
  egress_selection_changed: { title: '已手动选择线路', text: '管理员已选择备用线路作为当前出口。' },
  failover_pending: { title: '等待故障转移', text: '主线路发生异常，正在等待切换条件满足。' },
  ipv6_degraded: { title: 'IPv6 状态异常', text: 'IPv6 地址或 PD 前缀尚未就绪，IPv4 仍可正常使用。' },
  ipv6_recovered: { title: 'IPv6 已恢复', text: 'IPv6 地址和 PD 前缀已就绪。' },
  policy_synchronized: { title: '分流规则已同步', text: '防回环、运营商分流和域名分流规则已与当前出口同步。' },
  policy_conflict: { title: '发现规则冲突', text: '所选规则的配置已变化，请检查主备规则的匹配条件。' },
  cycle_failed: { title: '线路检查失败', text: '暂时无法读取爱快线路状态，将在下次检查时重试。' },
  cycle_recovered: { title: '线路检查已恢复', text: '已重新读取爱快线路状态，监测继续运行。' },
  configuration_saved: { title: '配置已保存', text: '新配置已写入，重启服务后生效。' },
  service_started: { title: '服务已启动', text: '故障转移服务已启动，开始监测线路和地址变化。' },
  service_restarting: { title: '服务正在重启', text: '正在重新启动故障转移服务以应用配置。' },
  service_stopped: { title: '服务已停止', text: '故障转移服务已停止，线路监测暂时暂停。' },
  service_failed: { title: '服务发生异常', text: '故障转移服务遇到错误，请检查日志中的错误记录。' },
}
