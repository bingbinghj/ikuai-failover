use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::{api::RouterApi, config::WanConfig, state::Egress};

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum Resource {
    FiveTupleRules,
    LoadBalanceRules,
    DomainRules,
    DomainRuleState,
    Ipv6Wan,
    Ipv6Lan,
}

impl Resource {
    pub fn path(self) -> &'static str {
        match self {
            Self::FiveTupleRules => "/routing/five-tuple-rules",
            Self::LoadBalanceRules => "/routing/load-balance-rules",
            Self::DomainRules | Self::DomainRuleState => "/routing/domain-rules",
            Self::Ipv6Wan => "/internal/ipv6/wan",
            Self::Ipv6Lan => "/internal/ipv6/lan",
        }
    }
    fn fields(self) -> &'static [&'static str] {
        match self {
            Self::FiveTupleRules => &[
                "id",
                "enabled",
                "tagname",
                "nexthop",
                "interface",
                "mode",
                "prio",
                "src_addr_inv",
                "src_addr",
                "dst_addr_inv",
                "dst_addr",
                "protocol",
                "src_port",
                "dst_port",
                "dst_type",
                "area_code",
                "type",
                "comment",
                "time",
                "iface_band",
            ],
            Self::LoadBalanceRules => &[
                "id",
                "enabled",
                "isp_name",
                "tagname",
                "mode",
                "comment",
                "weight",
                "interface",
            ],
            Self::DomainRules | Self::DomainRuleState => &[
                "id",
                "enabled",
                "tagname",
                "domain",
                "interface",
                "src_addr",
                "prio",
                "comment",
                "time",
            ],
            Self::Ipv6Wan => &[
                "id",
                "tagname",
                "enabled",
                "interface",
                "internet",
                "prefix",
                "prefix_hint",
                "force_prefix",
                "force_gen_duid",
            ],
            Self::Ipv6Lan => &[
                "id",
                "tagname",
                "enabled",
                "interface",
                "internet",
                "parent",
                "prefix_len",
                "dhcpv6",
                "ra_flags",
                "ra_static",
                "use_dns6",
                "ipv6_dns1",
                "ipv6_dns2",
                "leasetime",
                "ra_mtu_set",
                "ra_mtu",
            ],
        }
    }
    pub fn mutation_field(self) -> &'static str {
        match self {
            Self::DomainRules | Self::Ipv6Wan => "interface",
            Self::Ipv6Lan => "parent",
            _ => "enabled",
        }
    }
    pub fn is_ipv6(self) -> bool {
        matches!(self, Self::Ipv6Wan | Self::Ipv6Lan)
    }
    fn dynamic_field(self, key: &str) -> bool {
        match self {
            Self::Ipv6Wan => [
                "ipv6_addr",
                "ipv6_gateway",
                "dhcp6_ip_addr",
                "dhcp6_ip_gateway",
                "dhcp6_prefix1",
                "dhcp6_dns1",
                "dhcp6_dns2",
                "link_addr",
                "preferred_lft",
                "valid_lft",
            ]
            .contains(&key),
            Self::Ipv6Lan => ["ipv6_addr", "linkaddr", "preferred_lft", "valid_lft"].contains(&key),
            _ => false,
        }
    }
}

#[derive(Clone, Deserialize, Serialize)]
pub struct Change {
    pub resource: Resource,
    pub id: i64,
    pub before: Value,
    pub after: Value,
}

#[derive(Clone, Default, Serialize)]
pub struct Plan {
    pub target: Egress,
    pub changes: Vec<Change>,
    pub blockers: Vec<String>,
    pub ipv6_capability: String,
    pub ipv6_evidence: Option<crate::ipv6::Evidence>,
    pub ipv6_error: String,
}

pub fn normalized(resource: Resource, row: &Value) -> Result<Value, String> {
    let object = row
        .as_object()
        .ok_or_else(|| "Rule is not an object".to_string())?;
    let mut clean = serde_json::Map::new();
    for (key, value) in object {
        if key.ends_with("_int") || resource.dynamic_field(key) {
            continue;
        }
        if !resource.fields().contains(&key.as_str()) {
            return Err(format!("Unknown writable rule field: {key}"));
        }
        if resource == Resource::DomainRules
            && !["id", "tagname", "interface", "src_addr", "enabled"].contains(&key.as_str())
        {
            continue;
        }
        let mut value = value.clone();
        if [
            "src_addr", "dst_addr", "src_port", "dst_port", "time", "domain",
        ]
        .contains(&key.as_str())
        {
            if let Some(selector) = value.as_object_mut() {
                for child in ["custom", "object"] {
                    if selector
                        .get(child)
                        .and_then(Value::as_object)
                        .is_some_and(|v| v.is_empty())
                    {
                        selector.insert(child.into(), Value::Array(Vec::new()));
                    }
                }
            }
        }
        clean.insert(key.clone(), value);
    }
    Ok(Value::Object(clean))
}

fn traffic_selector(resource: Resource, row: &Value) -> Result<Value, String> {
    let mut selector = normalized(resource, row)?;
    let object = selector
        .as_object_mut()
        .ok_or_else(|| "Rule is not an object".to_string())?;
    for key in ["id", "tagname", "interface", "enabled", "comment"] {
        object.remove(key);
    }
    Ok(selector)
}

pub fn fingerprint(value: &Value) -> Result<String, String> {
    let bytes = serde_json::to_vec(value).map_err(|e| e.to_string())?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

pub fn physical_interface(value: &str, config: &WanConfig) -> bool {
    value == config.primary
        || value == config.backup
        || value == format!("{},{}", config.primary, config.backup)
        || value == format!("{},{}", config.backup, config.primary)
}

pub fn source_matches(row: &Value, source: &str) -> bool {
    row.get("src_addr")
        .and_then(|v| v.get("custom"))
        .and_then(Value::as_array)
        .is_some_and(|values| values.len() == 1 && values[0].as_str() == Some(source))
        && row
            .get("src_addr")
            .and_then(|v| v.get("object"))
            .is_some_and(|v| {
                v.as_object().is_some_and(|v| v.is_empty())
                    || v.as_array().is_some_and(|v| v.is_empty())
            })
}

pub fn full_domain_payload(row: &Value, desired: &Value) -> Result<Value, String> {
    let interface = row
        .get("interface")
        .and_then(Value::as_str)
        .ok_or_else(|| "Domain interface is absent".to_string())?;
    let interfaces: Vec<&str> = interface.split(',').collect();
    if interfaces.is_empty()
        || interfaces.len() > 2
        || !interfaces
            .iter()
            .all(|name| crate::config::is_wan_interface(name))
        || (interfaces.len() == 2 && interfaces[0] == interfaces[1])
    {
        return Err(
            "Domain rule was moved outside the managed physical interfaces before write".into(),
        );
    }
    let mut current = normalized(Resource::DomainRules, row)?;
    current["interface"] = desired
        .get("interface")
        .cloned()
        .ok_or_else(|| "Missing managed domain interface".to_string())?;
    if current != *desired {
        return Err("Domain identity, source or managed interface changed before write".into());
    }
    let mut full = row
        .as_object()
        .ok_or_else(|| "Domain row is not an object".to_string())?
        .clone();
    full.retain(|key, _| !key.ends_with("_int"));
    for key in ["src_addr", "domain", "time"] {
        if let Some(selector) = full.get_mut(key).and_then(Value::as_object_mut) {
            for child in ["custom", "object"] {
                if selector
                    .get(child)
                    .and_then(Value::as_object)
                    .is_some_and(|v| v.is_empty())
                {
                    selector.insert(child.into(), json_array());
                }
            }
        }
    }
    full.insert("interface".into(), desired["interface"].clone());
    Ok(Value::Object(full))
}

fn json_array() -> Value {
    Value::Array(Vec::new())
}

pub async fn rows(api: &dyn RouterApi, resource: Resource) -> Result<Vec<Value>, String> {
    let value = api.get(&format!("{}?limit=1000", resource.path())).await?;
    let rows = value
        .get("data")
        .and_then(Value::as_array)
        .ok_or_else(|| "API rule collection is missing data".to_string())?;
    if value
        .get("total")
        .and_then(Value::as_u64)
        .is_some_and(|total| total > rows.len() as u64)
    {
        return Err("Rule list is truncated; refusing partial policy management".into());
    }
    Ok(rows.to_vec())
}

pub async fn current(api: &dyn RouterApi, resource: Resource, id: i64) -> Result<Value, String> {
    let row = rows(api, resource)
        .await?
        .into_iter()
        .find(|row| row.get("id").and_then(Value::as_i64) == Some(id))
        .ok_or_else(|| format!("Configured rule id {id} is absent"))?;
    normalized(resource, &row)
}

pub async fn build(
    api: &dyn RouterApi,
    config: &WanConfig,
    target: Egress,
) -> Result<Plan, String> {
    let defaults = api.get("/interfaces/wan-config").await?;
    let data = defaults
        .get("data")
        .and_then(Value::as_array)
        .ok_or_else(|| "WAN configuration data is missing".to_string())?;
    let default_rows: Vec<&Value> = data
        .iter()
        .filter(|row| row.get("default_route").and_then(Value::as_i64) == Some(1))
        .collect();
    if [&config.primary, &config.backup, &config.preserved_default]
        .iter()
        .any(|name| {
            !data
                .iter()
                .any(|row| row.get("name").and_then(Value::as_str) == Some(name.as_str()))
        })
    {
        return Err("A selected WAN interface is absent; refresh the router configuration".into());
    }
    if default_rows.len() != 1
        || default_rows[0].get("name").and_then(Value::as_str)
            != Some(config.preserved_default.as_str())
    {
        return Err("The preserved default WAN route changed; policy management is blocked".into());
    }
    let mut plan = Plan {
        target,
        ..Plan::default()
    };
    plan.ipv6_capability = if config.ipv6_enabled {
        "required_missing"
    } else {
        "disabled"
    }
    .into();
    let mut cache = HashMap::new();
    for pair in &config.rule_pairs {
        if !cache.contains_key(&pair.resource) {
            cache.insert(pair.resource, rows(api, pair.resource).await?);
        }
        let records = cache
            .get(&pair.resource)
            .ok_or_else(|| "Missing policy snapshot".to_string())?;
        let primary = records
            .iter()
            .find(|row| row.get("id").and_then(Value::as_i64) == Some(pair.primary_id))
            .ok_or_else(|| format!("Configured rule id {} is absent", pair.primary_id))?;
        let backup = records
            .iter()
            .find(|row| row.get("id").and_then(Value::as_i64) == Some(pair.backup_id))
            .ok_or_else(|| format!("Configured rule id {} is absent", pair.backup_id))?;
        if traffic_selector(pair.resource, primary)? != traffic_selector(pair.resource, backup)? {
            return Err(format!(
                "Rule pair {} and {} no longer selects the same traffic",
                pair.primary_id, pair.backup_id
            ));
        }
        for (id, tag, interface, enabled) in [
            (
                pair.primary_id,
                pair.primary_tag.as_str(),
                config.primary.as_str(),
                target == Egress::Primary,
            ),
            (
                pair.backup_id,
                pair.backup_tag.as_str(),
                config.backup.as_str(),
                target == Egress::Backup,
            ),
        ] {
            let original = records
                .iter()
                .find(|row| row.get("id").and_then(Value::as_i64) == Some(id))
                .ok_or_else(|| format!("Configured rule id {id} is absent"))?;
            if original.get("tagname").and_then(Value::as_str) != Some(tag)
                || original.get("interface").and_then(Value::as_str) != Some(interface)
            {
                return Err(format!("Rule {id} identity or physical interface changed"));
            }
            let before = normalized(pair.resource, original)?;
            if !matches!(
                before.get("enabled").and_then(Value::as_str),
                Some("yes" | "no")
            ) {
                return Err("Unknown rule enabled format".into());
            }
            let mut after = before.clone();
            after["enabled"] = Value::String(if enabled { "yes" } else { "no" }.into());
            if before != after {
                plan.changes.push(Change {
                    resource: pair.resource,
                    id,
                    before,
                    after,
                });
            }
        }
    }
    // 先启用目标出口再关闭原出口，缩短分次API写入的断流窗口。
    // Enable the desired egress before disabling the previous one.
    plan.changes
        .sort_by_key(|change| change.after.get("enabled").and_then(Value::as_str) != Some("yes"));
    if !config.domain_rules.is_empty() {
        let records = rows(api, Resource::DomainRules).await?;
        for binding in &config.domain_rules {
            let row = records
                .iter()
                .find(|row| {
                    row.get("id").and_then(Value::as_i64) == Some(binding.id)
                        && row.get("tagname").and_then(Value::as_str) == Some(binding.tag.as_str())
                })
                .ok_or_else(|| "Explicit domain id/tag is absent".to_string())?;
            if row.get("enabled").and_then(Value::as_str) != Some("yes")
                || !source_matches(row, &binding.source)
                || !physical_interface(
                    row.get("interface")
                        .and_then(Value::as_str)
                        .unwrap_or_default(),
                    config,
                )
            {
                return Err(
                    "Domain binding enabled state, source or physical interface changed".into(),
                );
            }
            let before = normalized(Resource::DomainRules, row)?;
            let mut after = before.clone();
            after["interface"] = Value::String(
                if target == Egress::Primary {
                    &config.primary
                } else {
                    &config.backup
                }
                .clone(),
            );
            if before != after {
                plan.changes.push(Change {
                    resource: Resource::DomainRules,
                    id: binding.id,
                    before,
                    after,
                });
            }
        }
    }
    if config.ipv6_enabled {
        if let Err(error) = crate::ipv6::extend_plan(api, config, &mut plan).await {
            plan.ipv6_error = error;
        }
    }
    Ok(plan)
}
