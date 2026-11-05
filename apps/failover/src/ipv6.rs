use std::{net::Ipv6Addr, time::Duration};

use ipnet::{IpNet, Ipv6Net};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::time::Instant;

use crate::{
    api::RouterApi,
    config::{Ipv6Config, WanConfig},
    policy::{self, Change, Plan, Resource},
    state::Egress,
};

#[derive(Clone, Default, Deserialize, Serialize)]
pub struct Evidence {
    pub wan_address_ready: bool,
    pub delegated_prefix_ready: bool,
    pub lan_prefix_ready: bool,
}

impl Evidence {
    pub fn ready(&self) -> bool {
        self.wan_address_ready && self.delegated_prefix_ready && self.lan_prefix_ready
    }
}

pub struct Lease {
    address: Option<Ipv6Addr>,
    prefix: Option<Ipv6Net>,
}

pub fn lease(value: &Value) -> Lease {
    Lease {
        address: address(value, "dhcp6_ip_addr"),
        prefix: prefix(value),
    }
}

fn target(config: &WanConfig, egress: Egress) -> &str {
    if egress == Egress::Primary {
        &config.primary
    } else {
        &config.backup
    }
}

fn selected<'a>(rows: &'a [Value], id: i64, tag: &str) -> Result<&'a Value, String> {
    rows.iter()
        .find(|row| {
            row.get("id").and_then(Value::as_i64) == Some(id)
                && row.get("tagname").and_then(Value::as_str) == Some(tag)
        })
        .ok_or_else(|| "Explicit IPv6 id/tag is absent".into())
}

fn global(address: Ipv6Addr) -> bool {
    address.segments()[0] & 0xe000 == 0x2000
        && !(address.segments()[0] == 0x2001 && address.segments()[1] == 0x0db8)
}

fn address(value: &Value, field: &str) -> Option<Ipv6Addr> {
    let text = value.get(field)?.as_str()?;
    let network: IpNet = text.parse().ok()?;
    match network {
        IpNet::V6(net) if global(net.addr()) => Some(net.addr()),
        _ => None,
    }
}

pub fn prefix(value: &Value) -> Option<Ipv6Net> {
    let network: Ipv6Net = value.get("dhcp6_prefix1")?.as_str()?.parse().ok()?;
    if global(network.addr()) && network.prefix_len() <= 64 && network.prefix_len() > 0 {
        Some(network)
    } else {
        None
    }
}

fn lease_valid(value: &Value) -> bool {
    !matches!(value.get("valid_lft"), Some(Value::String(value)) if value == "0")
        && value.get("valid_lft").and_then(Value::as_u64) != Some(0)
}

fn evidence(wan: &Value, lan: &Value, interface: &str) -> Evidence {
    let correct_wan = wan.get("interface").and_then(Value::as_str) == Some(interface);
    let pd = prefix(wan).filter(|_| correct_wan && lease_valid(wan));
    let lan_addresses: Vec<&str> = lan
        .get("ipv6_addr")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .split(',')
        .filter(|text| !text.is_empty())
        .collect();
    let lan_good = lan.get("parent").and_then(Value::as_str) == Some(interface)
        && !lan_addresses.is_empty()
        && pd.is_some_and(|network| {
            lan_addresses.iter().all(|text| {
                text.parse::<Ipv6Net>().ok().is_some_and(|lan| {
                    global(lan.addr())
                        && network.contains(&lan.addr())
                        && lan.prefix_len() >= network.prefix_len()
                })
            })
        });
    Evidence {
        wan_address_ready: correct_wan
            && lease_valid(wan)
            && address(wan, "dhcp6_ip_addr").is_some(),
        delegated_prefix_ready: pd.is_some(),
        lan_prefix_ready: lan_good,
    }
}

pub async fn extend_plan(
    api: &dyn RouterApi,
    config: &WanConfig,
    plan: &mut Plan,
) -> Result<(), String> {
    if !api.ipv6_supported() {
        return Err(
            "IPv6 Web credentials are unavailable; required IPv6 synchronization blocks apply"
                .into(),
        );
    }
    let settings = config
        .ipv6
        .as_ref()
        .ok_or_else(|| "Explicit IPv6 WAN/LAN profile is missing".to_string())?;
    let capability = api.get("/internal/ipv6/capability").await?;
    if capability.get("multi_support").and_then(Value::as_u64) != Some(0)
        || capability.get("multi_num").and_then(Value::as_u64) != Some(1)
        || capability.get("bind_status").and_then(Value::as_u64) != Some(1)
    {
        return Err("IPv6 capability does not match the verified single-slot profile".into());
    }
    let wan_rows = policy::rows(api, Resource::Ipv6Wan).await?;
    let lan_rows = policy::rows(api, Resource::Ipv6Lan).await?;
    if wan_rows.len() != 1 {
        return Err("Single IPv6 WAN slot is required; no slot will be added".into());
    }
    let wan = selected(&wan_rows, settings.wan_id, &settings.wan_tag)?;
    let lan = selected(&lan_rows, settings.lan_id, &settings.lan_tag)?;
    let wan_interface = wan
        .get("interface")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if ![config.primary.as_str(), config.backup.as_str()].contains(&wan_interface)
        || wan.get("enabled").and_then(Value::as_str) != Some("yes")
        || wan.get("internet").and_then(Value::as_str) != Some("dhcp")
        || wan.get("prefix").and_then(Value::as_str) != Some("auto")
        || wan.get("force_prefix").and_then(Value::as_u64) != Some(0)
        || lan.get("interface").and_then(Value::as_str) != Some(settings.lan_interface.as_str())
        || lan.get("enabled").and_then(Value::as_str) != Some("yes")
        || lan.get("internet").and_then(Value::as_str) != Some("dhcp")
        || lan.get("prefix_len").and_then(Value::as_str) != Some("auto")
        || lan.get("parent").and_then(Value::as_str) != Some(wan_interface)
    {
        return Err(
            "IPv6 declaration changed or is not the verified automatic DHCP/PD profile".into(),
        );
    }
    let desired = target(config, plan.target);
    plan.ipv6_evidence = Some(evidence(wan, lan, desired));
    for (resource, original, field) in [
        (Resource::Ipv6Wan, wan, "interface"),
        (Resource::Ipv6Lan, lan, "parent"),
    ] {
        let before = policy::normalized(resource, original)?;
        let mut after = before.clone();
        after[field] = Value::String(desired.into());
        if before != after {
            let id = before
                .get("id")
                .and_then(Value::as_i64)
                .ok_or_else(|| "IPv6 id is invalid".to_string())?;
            plan.changes.push(Change {
                resource,
                id,
                before,
                after,
            });
        }
    }
    plan.ipv6_capability = "configuration_verified".into();
    Ok(())
}

pub async fn raw_wan(api: &dyn RouterApi, config: &Ipv6Config) -> Result<Value, String> {
    let rows = policy::rows(api, Resource::Ipv6Wan).await?;
    Ok(selected(&rows, config.wan_id, &config.wan_tag)?.clone())
}

pub async fn wait_wan(
    api: &dyn RouterApi,
    config: &Ipv6Config,
    interface: &str,
    previous: Option<Lease>,
    deadline: Instant,
) -> Result<(), String> {
    let mut last = Evidence::default();
    loop {
        let row = tokio::time::timeout_at(deadline,raw_wan(api,config)).await
            .map_err(|_| format!("IPv6 WAN settlement timeout: address_ready={}, delegated_prefix_ready={}; WAN address alone cannot satisfy LAN DHCP-PD",last.wan_address_ready,last.delegated_prefix_ready))??;
        last = evidence(&row, &Value::Null, interface);
        let current = prefix(&row);
        let fresh = previous.as_ref().is_none_or(|old| {
            (old.prefix.is_none() || current != old.prefix)
                && (old.address.is_none() || address(&row, "dhcp6_ip_addr") != old.address)
        });
        if last.wan_address_ready && last.delegated_prefix_ready && fresh {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "IPv6 WAN settlement timeout: address_ready={}, delegated_prefix_ready={}; required new LAN PD is unavailable",
                last.wan_address_ready, last.delegated_prefix_ready
            ));
        }
        tokio::time::sleep_until(
            (Instant::now() + Duration::from_millis(config.poll_interval_ms)).min(deadline),
        )
        .await;
    }
}

pub async fn wait_lan(
    api: &dyn RouterApi,
    config: &Ipv6Config,
    interface: &str,
    deadline: Instant,
) -> Result<(), String> {
    loop {
        let wan = tokio::time::timeout_at(deadline, raw_wan(api, config))
            .await
            .map_err(|_| "IPv6 LAN prefix settlement timeout".to_string())??;
        let rows = tokio::time::timeout_at(deadline, policy::rows(api, Resource::Ipv6Lan))
            .await
            .map_err(|_| "IPv6 LAN prefix settlement timeout".to_string())??;
        let lan = selected(&rows, config.lan_id, &config.lan_tag)?;
        let state = evidence(&wan, lan, interface);
        if state.wan_address_ready && state.delegated_prefix_ready && state.lan_prefix_ready {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err("IPv6 LAN did not obtain an address inside the new delegated prefix; configuration rollback is required".into());
        }
        tokio::time::sleep_until(
            (Instant::now() + Duration::from_millis(config.poll_interval_ms)).min(deadline),
        )
        .await;
    }
}
