use std::{
    collections::{BTreeMap, HashSet},
    sync::Arc,
    time::Duration,
};

use axum::{
    Json,
    extract::{State, rejection::JsonRejection},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
};
use serde::Serialize;
use serde_json::{Value, json};

use crate::{
    api::{Client, RouterApi},
    config::IkuaiConfig,
    policy::{self, Resource},
    runtime::Runtime,
};

#[derive(Serialize)]
struct InterfaceOption {
    name: String,
    display_name: String,
    label: String,
    kind: String,
    default_route: bool,
    comment: String,
}

#[derive(Serialize)]
struct RuleOption {
    id: i64,
    tag: String,
    comment: String,
    interface: String,
    enabled: bool,
    label: String,
    selector_fingerprint: String,
    selectable: bool,
    reason: String,
}

#[derive(Serialize)]
struct Ipv6Option {
    id: i64,
    tag: String,
    interface: String,
    parent: String,
    enabled: bool,
    internet: String,
    label: String,
    selectable: bool,
    reason: String,
}

#[derive(Serialize)]
struct Warning {
    section: &'static str,
    message: &'static str,
}

#[derive(Serialize)]
struct RuleOptions {
    five_tuple_rules: Vec<RuleOption>,
    load_balance_rules: Vec<RuleOption>,
    domain_rule_state: Vec<RuleOption>,
}

#[derive(Serialize)]
struct Ipv6Options {
    capability: Option<Value>,
    wan: Vec<Ipv6Option>,
    lan: Vec<Ipv6Option>,
}

#[derive(Serialize)]
struct Options {
    interfaces: Vec<InterfaceOption>,
    rules: RuleOptions,
    ipv6: Ipv6Options,
    warnings: Vec<Warning>,
}

/// Read choices through the running service's authenticated iKuai session.
pub async fn get(State(runtime): State<Arc<Runtime>>) -> Response {
    if !runtime.api.connection_configured() {
        return failure(
            StatusCode::BAD_REQUEST,
            "ikuai_credentials_missing",
            "请先填写爱快连接账号密码，再读取接口和规则。",
        );
    }
    response(runtime.api.as_ref()).await
}

/// Connect with the editor's unsaved connection fields; never save or apply rules.
pub async fn preview(
    State(_runtime): State<Arc<Runtime>>,
    request: Result<Json<IkuaiConfig>, JsonRejection>,
) -> Response {
    let Ok(Json(mut settings)) = request else {
        return invalid_settings();
    };
    settings.apply_environment();
    settings.base_url = settings.base_url.trim().to_string();
    if !valid_settings(&settings) {
        return invalid_settings();
    }
    if !settings.has_credentials() {
        return failure(
            StatusCode::BAD_REQUEST,
            "ikuai_credentials_missing",
            "请先填写爱快连接账号密码，再读取接口和规则。",
        );
    }
    let Ok(client) = Client::new(&settings) else {
        return invalid_settings();
    };
    response(&client).await
}

async fn response(api: &dyn RouterApi) -> Response {
    match tokio::time::timeout(Duration::from_secs(30), collect(api)).await {
        Ok(Ok(options)) => no_store(Json(options).into_response()),
        Ok(Err(())) => failure(
            StatusCode::BAD_GATEWAY,
            "ikuai_read_failed",
            "无法读取爱快接口，请检查连接地址、账号密码和网络。",
        ),
        Err(_) => failure(
            StatusCode::GATEWAY_TIMEOUT,
            "options_timeout",
            "读取爱快配置超时，请稍后重试。",
        ),
    }
}

async fn collect(api: &dyn RouterApi) -> Result<Options, ()> {
    // Interfaces must be authoritative; unavailable interfaces must not be invented.
    let snapshot = api.get("/internal/interfaces").await.map_err(|_| ())?;
    let interface_rows = complete_rows(&snapshot).map_err(|_| ())?;
    let mut warnings = Vec::new();
    let names = match api.get("/monitoring/wan-information").await {
        Ok(value) => value
            .get("data")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|row| Some((text(row, "interface", 64), text(row, "display_name", 256))))
            .filter(|(name, display)| !name.is_empty() && !display.is_empty())
            .collect::<BTreeMap<_, _>>(),
        Err(_) => {
            warnings.push(Warning {
                section: "interfaces",
                message: "运营商标识暂不可用，接口仍可按名称选择。",
            });
            BTreeMap::new()
        }
    };
    let mut seen = HashSet::new();
    let mut interfaces = Vec::new();
    for row in interface_rows {
        let name = text(row, "name", 64);
        let kind = text(row, "kind", 8);
        if name.is_empty() || !["wan", "lan"].contains(&kind.as_str()) || !seen.insert(name.clone())
        {
            return Err(());
        }
        let display_name = names.get(&name).cloned().unwrap_or_default();
        let label = interface_label(&name, &display_name);
        let default_route = row.get("default_route").and_then(Value::as_i64) == Some(1);
        interfaces.push(InterfaceOption {
            name,
            display_name,
            label,
            kind,
            default_route,
            comment: text(row, "comment", 512),
        });
    }
    if !interfaces.iter().any(|interface| interface.kind == "wan") {
        return Err(());
    }
    let wan_names: HashSet<&str> = interfaces
        .iter()
        .filter(|interface| interface.kind == "wan")
        .map(|interface| interface.name.as_str())
        .collect();

    let (five_tuple, load_balance, domain, capability, ipv6_wan, ipv6_lan) = tokio::join!(
        api.get(Resource::FiveTupleRules.path()),
        api.get(Resource::LoadBalanceRules.path()),
        api.get(Resource::DomainRuleState.path()),
        api.get("/internal/ipv6/capability"),
        api.get(Resource::Ipv6Wan.path()),
        api.get(Resource::Ipv6Lan.path()),
    );
    let rules = RuleOptions {
        five_tuple_rules: rules(
            five_tuple,
            Resource::FiveTupleRules,
            &wan_names,
            &names,
            "five_tuple_rules",
            &mut warnings,
        ),
        load_balance_rules: rules(
            load_balance,
            Resource::LoadBalanceRules,
            &wan_names,
            &names,
            "load_balance_rules",
            &mut warnings,
        ),
        domain_rule_state: rules(
            domain,
            Resource::DomainRuleState,
            &wan_names,
            &names,
            "domain_rule_state",
            &mut warnings,
        ),
    };
    let capability = match capability {
        Ok(value) => Some(json!({"multi_support":value.get("multi_support"),
            "multi_num":value.get("multi_num"),"bind_status":value.get("bind_status")})),
        Err(_) => {
            warnings.push(Warning {
                section: "ipv6",
                message: "无法读取 IPv6 能力，IPv4 选项仍可使用。",
            });
            None
        }
    };
    let supported = capability.as_ref().is_some_and(|value| {
        value.get("multi_support").and_then(Value::as_u64) == Some(0)
            && value.get("multi_num").and_then(Value::as_u64) == Some(1)
            && value.get("bind_status").and_then(Value::as_u64) == Some(1)
    });
    let wan = ipv6_options(
        ipv6_wan,
        Resource::Ipv6Wan,
        supported,
        &names,
        &mut warnings,
    );
    let lan = ipv6_options(
        ipv6_lan,
        Resource::Ipv6Lan,
        supported,
        &names,
        &mut warnings,
    );
    Ok(Options {
        interfaces,
        rules,
        ipv6: Ipv6Options {
            capability,
            wan,
            lan,
        },
        warnings,
    })
}

fn complete_rows(value: &Value) -> Result<&[Value], ()> {
    let rows = value.get("data").and_then(Value::as_array).ok_or(())?;
    let total = value.get("total").and_then(Value::as_u64).ok_or(())?;
    if total > rows.len() as u64 || rows.len() > 1000 {
        return Err(());
    }
    Ok(rows)
}

fn rules(
    result: Result<Value, String>,
    resource: Resource,
    interfaces: &HashSet<&str>,
    names: &BTreeMap<String, String>,
    section: &'static str,
    warnings: &mut Vec<Warning>,
) -> Vec<RuleOption> {
    let Ok(value) = result else {
        warnings.push(Warning {
            section,
            message: "此类规则暂未读取成功，请刷新爱快选项。",
        });
        return Vec::new();
    };
    let Ok(rows) = complete_rows(&value) else {
        warnings.push(Warning {
            section,
            message: "规则列表不完整，暂不提供该类规则选择。",
        });
        return Vec::new();
    };
    let mut seen = HashSet::new();
    let mut result = Vec::new();
    for row in rows {
        let Some(id) = row.get("id").and_then(Value::as_i64).filter(|id| *id > 0) else {
            continue;
        };
        if !seen.insert(id) {
            continue;
        }
        let tag = text(row, "tagname", 256);
        let comment = text(row, "comment", 512);
        let interface = text(row, "interface", 64);
        let enabled = row.get("enabled").and_then(Value::as_str);
        let mut reason = if tag.is_empty() {
            "规则未设置名称，无法可靠识别。"
        } else if !interfaces.contains(interface.as_str()) {
            "规则未固定到单个可用 WAN 接口。"
        } else if !matches!(enabled, Some("yes" | "no")) {
            "规则启用状态格式不受支持。"
        } else {
            ""
        };
        let selector_fingerprint = match policy::normalized(resource, row) {
            Ok(mut selector) => {
                if let Some(object) = selector.as_object_mut() {
                    for key in ["id", "tagname", "interface", "enabled", "comment"] {
                        object.remove(key);
                    }
                }
                policy::fingerprint(&selector).unwrap_or_default()
            }
            Err(_) => {
                reason = "此规则字段与当前支持的模型不匹配。";
                String::new()
            }
        };
        let name = if tag.is_empty() {
            if comment.is_empty() {
                "未命名规则"
            } else {
                &comment
            }
        } else {
            &tag
        };
        let interface_title = interface_label(
            &interface,
            names
                .get(&interface)
                .map(String::as_str)
                .unwrap_or_default(),
        );
        let label = if interface.is_empty() {
            name.to_string()
        } else {
            format!("{name} · {interface_title}")
        };
        result.push(RuleOption {
            id,
            tag,
            comment,
            interface,
            enabled: enabled == Some("yes"),
            label,
            selectable: reason.is_empty() && !selector_fingerprint.is_empty(),
            selector_fingerprint,
            reason: reason.into(),
        });
    }
    result
}

fn ipv6_options(
    result: Result<Value, String>,
    resource: Resource,
    supported: bool,
    names: &BTreeMap<String, String>,
    warnings: &mut Vec<Warning>,
) -> Vec<Ipv6Option> {
    let Ok(value) = result else {
        warnings.push(Warning {
            section: "ipv6",
            message: "IPv6 实例暂未读取成功，IPv4 选项仍可使用。",
        });
        return Vec::new();
    };
    let Ok(rows) = complete_rows(&value) else {
        warnings.push(Warning {
            section: "ipv6",
            message: "IPv6 实例列表不完整，暂不提供实例选择。",
        });
        return Vec::new();
    };
    let mut seen = HashSet::new();
    rows.iter()
        .filter_map(|row| {
            let id = row.get("id").and_then(Value::as_i64).filter(|id| *id > 0)?;
            if !seen.insert(id) {
                return None;
            }
            let tag = text(row, "tagname", 256);
            let interface = text(row, "interface", 64);
            let parent = text(row, "parent", 64);
            let internet = text(row, "internet", 16);
            let enabled = row.get("enabled").and_then(Value::as_str) == Some("yes");
            let reason = if !supported || (resource == Resource::Ipv6Wan && rows.len() != 1) {
                "当前仅支持单 WAN 实例的自动 DHCP/PD 模型。"
            } else if tag.is_empty() || interface.is_empty() {
                "实例名称或接口信息不完整。"
            } else if !enabled || internet != "dhcp" {
                "实例需要启用自动 DHCP 模式。"
            } else if resource == Resource::Ipv6Wan
                && (row.get("prefix").and_then(Value::as_str) != Some("auto")
                    || row.get("force_prefix").and_then(Value::as_u64) != Some(0))
            {
                "WAN 实例需要使用自动前缀。"
            } else if resource == Resource::Ipv6Lan
                && row.get("prefix_len").and_then(Value::as_str) != Some("auto")
            {
                "LAN 实例需要使用自动前缀。"
            } else if policy::normalized(resource, row).is_err() {
                "此实例字段与当前支持的模型不匹配。"
            } else {
                ""
            };
            let interface_title = interface_label(
                &interface,
                names
                    .get(&interface)
                    .map(String::as_str)
                    .unwrap_or_default(),
            );
            let label = format!(
                "{} · {interface_title}",
                if tag.is_empty() {
                    "未命名实例"
                } else {
                    &tag
                }
            );
            Some(Ipv6Option {
                id,
                tag,
                interface,
                parent,
                enabled,
                internet,
                label,
                selectable: reason.is_empty(),
                reason: reason.into(),
            })
        })
        .collect()
}

fn text(row: &Value, field: &str, limit: usize) -> String {
    row.get(field)
        .and_then(Value::as_str)
        .filter(|text| text.len() <= limit && !text.chars().any(char::is_control))
        .unwrap_or_default()
        .to_string()
}

fn interface_label(interface: &str, display: &str) -> String {
    if display.is_empty() {
        interface.to_uppercase()
    } else {
        format!("{display} · {}", interface.to_uppercase())
    }
}

fn valid_settings(settings: &IkuaiConfig) -> bool {
    settings.base_url.len() <= 2048
        && settings.username.len() <= 256
        && settings.password.len() <= 1024
        && !settings.username.chars().any(char::is_control)
        && !settings.password.chars().any(char::is_control)
        && (1..=30_000).contains(&settings.timeout_ms)
        && reqwest::Url::parse(settings.base_url.trim()).is_ok_and(|url| {
            url.scheme() == "https"
                && url.host_str().is_some()
                && url.username().is_empty()
                && url.password().is_none()
                && url.query().is_none()
                && url.fragment().is_none()
        })
}

fn invalid_settings() -> Response {
    failure(
        StatusCode::BAD_REQUEST,
        "invalid_connection_settings",
        "请填写有效的爱快 HTTPS 地址、连接账号密码和超时时间。",
    )
}

fn failure(status: StatusCode, code: &'static str, message: &'static str) -> Response {
    no_store(
        (
            status,
            Json(json!({"success":false,"error":code,"message":message})),
        )
            .into_response(),
    )
}

fn no_store(mut response: Response) -> Response {
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        axum::http::HeaderValue::from_static("no-store"),
    );
    response
}
