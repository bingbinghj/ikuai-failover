use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Default, Debug, Deserialize, Serialize)]
#[serde(default)]
pub struct LineContext {
    pub interface: String,
    pub isp: String,
    pub ipv4: String,
    pub ipv6: String,
    pub prefix: String,
}

#[derive(Clone, Default, Debug, Deserialize, Serialize)]
#[serde(default)]
pub struct NetworkContext {
    pub interface: String,
    pub isp: String,
    pub ipv4: String,
    pub ipv6: String,
    pub prefix: String,
    pub lines: Vec<LineContext>,
    pub stale: bool,
    #[serde(skip)]
    pub previous: Option<Box<NetworkContext>>,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub affected_interface: String,
}

fn get(value: &Value) -> String {
    value.as_str().unwrap_or_default().to_string()
}

impl NetworkContext {
    pub fn from_snapshot(snapshot: &Value) -> Self {
        let primary = snapshot["primary_interface"].as_str().unwrap_or("wan1");
        let backup = snapshot["backup_interface"].as_str().unwrap_or("wan3");
        let preserved = snapshot["preserved_default"].as_str().unwrap_or("wan2");
        let active = if snapshot
            .pointer("/wan/state/active")
            .and_then(Value::as_str)
            == Some("backup")
        {
            backup
        } else {
            primary
        };
        let mut lines = Vec::new();
        for interface in [primary, backup, preserved] {
            if lines
                .iter()
                .any(|line: &LineContext| line.interface == interface)
            {
                continue;
            }
            let ipv6 = snapshot["ipv6_addresses"].as_array().and_then(|rows| {
                rows.iter()
                    .find(|row| row["interface"].as_str() == Some(interface))
            });
            lines.push(LineContext {
                interface: interface.to_uppercase(),
                isp: get(&snapshot["interface_names"][interface]),
                ipv4: get(&snapshot["wan"]["interface_addresses"][interface]),
                ipv6: ipv6.map_or_else(String::new, |row| get(&row["dhcp6_ip_addr"])),
                prefix: ipv6.map_or_else(String::new, |row| get(&row["dhcp6_prefix1"])),
            });
        }
        let current = lines
            .iter()
            .find(|line| line.interface.eq_ignore_ascii_case(active))
            .cloned()
            .unwrap_or_default();
        Self {
            interface: current.interface,
            isp: current.isp,
            ipv4: current.ipv4,
            ipv6: current.ipv6,
            prefix: current.prefix,
            lines,
            stale: snapshot["initializing"].as_bool().unwrap_or(false)
                || snapshot["ipv6_addresses_stale"].as_bool().unwrap_or(false),
            previous: None,
            affected_interface: String::new(),
        }
    }

    pub fn with_previous(mut self, previous: &Self) -> Self {
        if self.stale {
            for line in &mut self.lines {
                if let Some(old) = previous
                    .lines
                    .iter()
                    .find(|old| old.interface == line.interface)
                {
                    for (current, saved) in [
                        (&mut line.isp, &old.isp),
                        (&mut line.ipv4, &old.ipv4),
                        (&mut line.ipv6, &old.ipv6),
                        (&mut line.prefix, &old.prefix),
                    ] {
                        if current.is_empty() {
                            *current = saved.clone();
                        }
                    }
                }
            }
            if let Some(line) = self
                .lines
                .iter()
                .find(|line| line.interface == self.interface)
            {
                self.isp = line.isp.clone();
                self.ipv4 = line.ipv4.clone();
                self.ipv6 = line.ipv6.clone();
                self.prefix = line.prefix.clone();
            }
        }
        self
    }

    pub fn with_transition(mut self, previous: &Self, affected_interface: &str) -> Self {
        let mut previous = previous.clone();
        previous.previous = None;
        previous.affected_interface.clear();
        self.previous = Some(Box::new(previous));
        self.affected_interface = affected_interface.to_string();
        self
    }

    pub fn transition_lines(&self) -> (LineContext, LineContext) {
        let select = |context: &Self| {
            if !self.affected_interface.is_empty() {
                if let Some(line) = context.lines.iter().find(|line| {
                    line.interface
                        .eq_ignore_ascii_case(&self.affected_interface)
                }) {
                    return normalized_line(line.clone());
                }
                if !context
                    .interface
                    .eq_ignore_ascii_case(&self.affected_interface)
                {
                    return LineContext::default();
                }
            }
            context.active_line()
        };
        (
            self.previous.as_deref().map(select).unwrap_or_default(),
            select(self),
        )
    }

    pub fn active_line(&self) -> LineContext {
        normalized_line(LineContext {
            interface: self.interface.clone(),
            isp: self.isp.clone(),
            ipv4: self.ipv4.clone(),
            ipv6: self.ipv6.clone(),
            prefix: self.prefix.clone(),
        })
    }
}

pub fn current_summary(context: &NetworkContext) -> String {
    let mut lines = Vec::new();
    let current_identity = identity(&context.interface, &context.isp);
    if !current_identity.is_empty() {
        lines.push(format!("🌐 当前接口：{current_identity}"));
    }
    append_address(&mut lines, "🌍 IPv4", &context.ipv4);
    append_address(&mut lines, "🌐 IPv6", &context.ipv6);
    append_address(&mut lines, "📡 PD 前缀", &context.prefix);
    for line in &context.lines {
        if line.interface.eq_ignore_ascii_case(&context.interface) {
            continue;
        }
        let line_identity = identity(&line.interface, &line.isp);
        if !line_identity.is_empty() {
            lines.push(format!("🔌 {line_identity}"));
        }
        append_address(&mut lines, "   IPv4", &line.ipv4);
        append_address(&mut lines, "   IPv6", &line.ipv6);
        append_address(&mut lines, "   PD 前缀", &line.prefix);
    }
    if context.stale
        && context
            .lines
            .iter()
            .any(|line| available(&line.ipv4) || available(&line.ipv6) || available(&line.prefix))
    {
        lines.push("⏳ 最新状态待检测；已有地址为上次记录。".into());
    }
    lines.join("\n")
}

fn identity(interface: &str, isp: &str) -> String {
    [interface.trim(), isp.trim()]
        .into_iter()
        .filter(|value| available(value))
        .collect::<Vec<_>>()
        .join(" · ")
}

fn append_address(rows: &mut Vec<String>, label: &str, value: &str) {
    if available(value) {
        rows.push(format!("{label}：{value}"));
    }
}

pub fn available(value: &str) -> bool {
    let value = value.trim();
    !value.is_empty()
        && !matches!(
            value.to_ascii_lowercase().as_str(),
            "unknown"
                | "unavailable"
                | "none"
                | "null"
                | "n/a"
                | "-"
                | "—"
                | "尚未获取"
                | "未获取"
                | "0.0.0.0"
                | "::"
                | "::/0"
        )
}

fn normalized_line(mut line: LineContext) -> LineContext {
    for value in [
        &mut line.interface,
        &mut line.isp,
        &mut line.ipv4,
        &mut line.ipv6,
        &mut line.prefix,
    ] {
        if !available(value) {
            value.clear();
        }
    }
    line
}

pub fn transition_kind(kind: &str) -> bool {
    matches!(
        kind,
        "egress_switched"
            | "egress_recovered"
            | "egress_selection_changed"
            | "wan_health_changed"
            | "wan_ipv4_changed"
            | "wan_ipv6_changed"
            | "ipv6_pd_changed"
            | "ipv6_recovered"
            | "cycle_recovered"
    )
}

pub fn transition_summary(context: &NetworkContext) -> String {
    if context.previous.is_none() {
        return String::new();
    }
    let (before, after) = context.transition_lines();
    let mut sections = Vec::new();
    for (label, line) in [("之前", before), ("之后", after)] {
        let identity = identity(&line.interface, &line.isp);
        let mut rows = Vec::new();
        append_address(&mut rows, "   IPv4", &line.ipv4);
        append_address(&mut rows, "   IPv6", &line.ipv6);
        append_address(&mut rows, "   PD 前缀", &line.prefix);
        if identity.is_empty() && rows.is_empty() {
            continue;
        }
        rows.insert(
            0,
            if identity.is_empty() {
                label.to_string()
            } else {
                format!("{label}：{identity}")
            },
        );
        sections.push(rows.join("\n"));
    }
    if sections.is_empty() {
        String::new()
    } else {
        format!("🔀 前后线路信息\n{}", sections.join("\n   ↓\n"))
    }
}

pub fn summary(context: &NetworkContext) -> String {
    let transition = transition_summary(context);
    let current = current_summary(context);
    if transition.is_empty() {
        current
    } else {
        format!("{transition}\n\n{current}")
    }
}
