pub const EVENTS: &[(&str, &str)] = &[
    ("wan_health_changed", "线路状态变化"),
    ("wan_ipv4_changed", "IPv4 地址变化"),
    ("wan_ipv6_changed", "IPv6 地址变化"),
    ("ipv6_pd_changed", "PD 前缀变化"),
    ("egress_switched", "出口切换"),
    ("egress_recovered", "恢复主线路"),
    ("egress_selection_changed", "手动选择线路"),
    ("failover_pending", "等待故障转移"),
    ("ipv6_degraded", "IPv6 异常"),
    ("ipv6_recovered", "IPv6 恢复"),
    ("policy_synchronized", "规则已同步"),
    ("policy_conflict", "规则冲突"),
    ("cycle_failed", "线路检查失败"),
    ("cycle_recovered", "线路检查恢复"),
    ("configuration_saved", "配置已保存"),
    ("service_started", "服务启动"),
    ("service_restarting", "服务重启"),
    ("service_stopped", "服务停止"),
    ("service_failed", "服务异常"),
];

pub fn known(kind: &str) -> bool {
    EVENTS.iter().any(|(event, _)| *event == kind)
}

pub fn label(kind: &str) -> &str {
    EVENTS
        .iter()
        .find(|(event, _)| *event == kind)
        .map_or(kind, |(_, label)| label)
}

pub fn deserialize_selection<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Vec<String>, D::Error> {
    let values = <Vec<String> as serde::Deserialize>::deserialize(deserializer)?;
    let legacy = [
        "资源下载",
        "整理入库",
        "订阅",
        "站点",
        "媒体服务器",
        "手动处理",
        "插件",
        "智能体",
        "其它",
    ];
    let mut migrated = Vec::new();
    for value in values {
        if legacy.contains(&value.as_str()) {
            continue;
        }
        if !migrated.contains(&value) {
            migrated.push(value);
        }
    }
    Ok(migrated)
}
