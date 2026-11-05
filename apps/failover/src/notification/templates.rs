use minijinja::{Environment, UndefinedBehavior, context};
use serde_json::Value;

use super::{Event, NotificationError};
use std::collections::BTreeMap;

const MAX_TEMPLATE_BYTES: usize = 16 * 1024;
const MAX_RENDERED_BYTES: usize = 32 * 1024;

fn emoji(kind: &str) -> &'static str {
    match kind {
        "wan_health_changed" => "🌐",
        "wan_ipv4_changed" => "🌍",
        "wan_ipv6_changed" => "🌐",
        "ipv6_pd_changed" => "📡",
        "egress_switched" | "egress_selection_changed" => "🔀",
        "egress_recovered" | "ipv6_recovered" | "cycle_recovered" => "✅",
        "failover_pending" => "⏳",
        "policy_synchronized" => "🛠️",
        "configuration_saved" => "💾",
        "service_started" => "🚀",
        "service_restarting" => "🔄",
        "service_stopped" => "⏹️",
        "policy_conflict" | "cycle_failed" | "service_failed" | "ipv6_degraded" => "⚠️",
        _ => "🔔",
    }
}

pub fn category(kind: &str) -> &'static str {
    match kind {
        "wan_health_changed" | "wan_ipv4_changed" | "wan_ipv6_changed" | "ipv6_pd_changed" => {
            "network_change"
        }
        "egress_switched"
        | "egress_recovered"
        | "egress_selection_changed"
        | "failover_pending"
        | "ipv6_degraded"
        | "ipv6_recovered" => "egress_change",
        "policy_synchronized"
        | "policy_conflict"
        | "cycle_failed"
        | "cycle_recovered"
        | "configuration_saved" => "policy_change",
        _ => "service_change",
    }
}

pub fn defaults() -> BTreeMap<String, String> {
    let body = "{% if text %}{{ text }}{% endif %}{% if transition_summary %}\n\n{{ transition_summary }}{% endif %}{% if network_summary %}\n\n{{ network_summary }}{% endif %}\n🕒 时间：{{ timestamp }}";
    [
        "default",
        "network_change",
        "egress_change",
        "policy_change",
        "service_change",
    ]
    .into_iter()
    .map(|kind| {
        (
            kind.into(),
            serde_json::json!({"title":"{{ event_icon }} {{ title }}","text":body}).to_string(),
        )
    })
    .collect()
}

pub fn configured_defaults() -> BTreeMap<String, String> {
    BTreeMap::from([(
        "default".into(),
        defaults().remove("default").unwrap_or_default(),
    )])
}

pub fn select(templates: &BTreeMap<String, String>, kind: &str) -> String {
    templates
        .get(category(kind))
        .or_else(|| templates.get(kind))
        .or_else(|| {
            if matches!(
                kind,
                "cycle_failed" | "cycle_recovered" | "configuration_saved"
            ) {
                templates.get("service_change")
            } else {
                None
            }
        })
        .or_else(|| templates.get("default"))
        .cloned()
        .unwrap_or_else(|| defaults().remove(category(kind)).unwrap_or_default())
}

pub(crate) fn notification_timestamp(timestamp_ms: u64) -> String {
    crate::logging::timestamp(timestamp_ms.saturating_add(8 * 60 * 60 * 1000))
        .replace('T', " ")
        .replace('Z', "")
}

struct LimitedOutput(Vec<u8>);
impl std::io::Write for LimitedOutput {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if self.0.len().saturating_add(bytes.len()) > MAX_RENDERED_BYTES {
            return Err(std::io::Error::other("Template output exceeds its limit"));
        }
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn document(source: &str) -> Result<Value, NotificationError> {
    if source.len() > MAX_TEMPLATE_BYTES {
        return Err(NotificationError::InvalidConfiguration);
    }
    let value: Value =
        serde_json::from_str(source).map_err(|_| NotificationError::InvalidConfiguration)?;
    let object = value
        .as_object()
        .ok_or(NotificationError::InvalidConfiguration)?;
    if object
        .keys()
        .any(|key| !["title", "text"].contains(&key.as_str()))
        || object.values().any(|value| !value.is_string())
    {
        return Err(NotificationError::InvalidConfiguration);
    }
    Ok(value)
}

pub fn validate(source: &str) -> Result<(), NotificationError> {
    let value = document(source)?;
    let environment = Environment::new();
    for template in value.as_object().unwrap().values() {
        environment
            .template_from_str(template.as_str().unwrap())
            .map_err(|_| NotificationError::InvalidConfiguration)?;
    }
    Ok(())
}

pub fn render(source: &str, event: &Event) -> Result<Event, NotificationError> {
    let value = document(source)?;
    let mut environment = Environment::new();
    environment.set_fuel(Some(10_000));
    environment.set_undefined_behavior(UndefinedBehavior::Strict);
    let mut result = event.clone();
    let resolved_context = if let Some(previous) = event.previous_context.as_ref() {
        event
            .context
            .clone()
            .with_transition(previous, &event.context.affected_interface)
    } else if super::network_context::transition_kind(&event.kind)
        && event.context.previous.is_none()
    {
        event
            .context
            .clone()
            .with_transition(&Default::default(), &event.context.affected_interface)
    } else {
        event.context.clone()
    };
    let network_summary = super::network_context::current_summary(&resolved_context);
    let transition_summary = super::network_context::transition_summary(&resolved_context);
    let (from, to) = resolved_context.transition_lines();
    let current = resolved_context.active_line();
    result.context = resolved_context;
    for (key, value) in value.as_object().unwrap() {
        let template = environment
            .template_from_str(value.as_str().unwrap())
            .map_err(|_| NotificationError::InvalidConfiguration)?;
        let timestamp = notification_timestamp(event.timestamp_ms);
        let context = context! { title => &event.title, text => &event.text, message => &event.text,
        kind => &event.kind, event_type => &event.kind, event_label => super::event_types::label(&event.kind),
        event_icon => emoji(&event.kind), service => "ikuai-failover", timestamp => &timestamp, time => &timestamp,
        timestamp_ms => event.timestamp_ms, event => event, interface => &current.interface,
        isp => &current.isp, ipv4 => &current.ipv4, ipv6 => &current.ipv6, prefix => &current.prefix,
        lines => &event.context.lines, network_summary => &network_summary, transition_summary => &transition_summary,
        from => &from, to => &to, has_previous => !from.interface.is_empty(),
        from_interface => &from.interface, to_interface => &to.interface, from_isp => &from.isp, to_isp => &to.isp,
        from_ipv4 => &from.ipv4, to_ipv4 => &to.ipv4, from_ipv6 => &from.ipv6, to_ipv6 => &to.ipv6,
        from_prefix => &from.prefix, to_prefix => &to.prefix };
        let mut output = LimitedOutput(Vec::new());
        template
            .render_captured_to(context, &mut output)
            .map_err(|_| NotificationError::InvalidConfiguration)?;
        let rendered =
            String::from_utf8(output.0).map_err(|_| NotificationError::InvalidConfiguration)?;
        match key.as_str() {
            "title" => result.title = rendered,
            "text" => result.text = rendered,
            _ => unreachable!(),
        }
    }
    // Network metadata and local time are always present, including custom templates.
    result.text = omit_empty_network_rows(&result.text);
    for summary in [&transition_summary, &network_summary] {
        if !summary.is_empty() && !result.text.contains(summary.as_str()) {
            result.text.push_str(&format!("\n\n{summary}"));
        }
    }
    let timestamp = notification_timestamp(event.timestamp_ms);
    if !result.text.contains(&timestamp) {
        result.text.push_str(&format!("\n🕒 时间：{timestamp}"));
    }
    if result.text.len() > MAX_RENDERED_BYTES {
        return Err(NotificationError::InvalidConfiguration);
    }
    if result.title.trim().is_empty() && result.text.trim().is_empty() {
        return Err(NotificationError::InvalidConfiguration);
    }
    Ok(result)
}

fn omit_empty_network_rows(text: &str) -> String {
    text.lines()
        .filter(|line| {
            let trimmed = line
                .trim()
                .trim_matches(|character| matches!(character, '*' | '_' | '`' | '>' | '#' | ' '));
            let Some((label, value)) = trimmed.split_once('：').or_else(|| trimmed.split_once(':'))
            else {
                return true;
            };
            let label = label
                .trim()
                .trim_matches(|character| matches!(character, '*' | '_' | '`' | ' '))
                .trim_start_matches(|character: char| !character.is_alphanumeric());
            let network_label = [
                "IPv4",
                "IPv6",
                "PD",
                "PD 前缀",
                "接口",
                "当前接口",
                "运营商",
                "之前",
                "之后",
                "切换前",
                "切换后",
                "之前接口",
                "之后接口",
                "之前运营商",
                "之后运营商",
                "切换前接口",
                "切换后接口",
                "切换前运营商",
                "切换后运营商",
                "之前 IPv4",
                "之后 IPv4",
                "之前 IPv6",
                "之后 IPv6",
                "之前 PD 前缀",
                "之后 PD 前缀",
            ]
            .contains(&label);
            let value = value
                .trim()
                .trim_matches(|character| matches!(character, '*' | '_' | '`' | ' '));
            !network_label || super::network_context::available(value)
        })
        .fold(Vec::<&str>::new(), |mut rows, line| {
            if !line.trim().is_empty()
                || rows
                    .last()
                    .is_none_or(|previous| !previous.trim().is_empty())
            {
                rows.push(line);
            }
            rows
        })
        .join("\n")
        .trim()
        .to_string()
}
