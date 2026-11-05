use base64::Engine;
use futures_util::{StreamExt, stream};
use serde_json::{Value, json};
use std::{
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};
use tokio::sync::Semaphore;

use crate::{
    config::IkuaiConfig,
    policy::{self, Resource},
};

const MAX_RESPONSE_BYTES: usize = 16 * 1024 * 1024;

pub struct WebClient {
    client: reqwest::Client,
    base: String,
    username: String,
    password: String,
    authenticated: AtomicBool,
    login_gate: Semaphore,
    timeout: Duration,
}

enum CallError {
    SessionExpired,
    Request(String),
}

impl CallError {
    fn message(self) -> String {
        match self {
            Self::SessionExpired => {
                "iKuai Web session expired; the request was not replayed".into()
            }
            Self::Request(message) => message,
        }
    }
}

impl WebClient {
    pub fn new(settings: &IkuaiConfig) -> Result<Self, String> {
        Self::with_credentials(
            settings,
            settings.username.clone(),
            settings.password.clone(),
        )
    }

    pub fn with_credentials(
        settings: &IkuaiConfig,
        username: String,
        password: String,
    ) -> Result<Self, String> {
        if username.is_empty() || password.is_empty() {
            return Err("An iKuai username and password are required".into());
        }
        let timeout = Duration::from_millis(settings.timeout_ms);
        let client = reqwest::Client::builder()
            .cookie_store(true)
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .danger_accept_invalid_certs(settings.allow_invalid_certificate)
            .connect_timeout(timeout)
            .timeout(timeout)
            .build()
            .map_err(|_| "The iKuai Web client could not be initialized".to_string())?;
        Ok(Self {
            client,
            base: settings.base_url.trim_end_matches('/').into(),
            username,
            password,
            authenticated: AtomicBool::new(false),
            login_gate: Semaphore::new(1),
            timeout,
        })
    }

    async fn login(&self) -> Result<(), String> {
        if self.authenticated.load(Ordering::SeqCst) {
            return Ok(());
        }
        let permit = tokio::time::timeout(self.timeout, self.login_gate.acquire())
            .await
            .map_err(|_| "Web authentication lock timeout".to_string())?
            .map_err(|_| "Web authentication is unavailable".to_string())?;
        if !self.authenticated.load(Ordering::SeqCst) {
            let body = json!({"username":self.username,"passwd":format!("{:x}",md5::compute(&self.password)),
                "pass":base64::engine::general_purpose::STANDARD.encode(format!("salt_11{}",self.password)),"remember_password":""});
            let response = self
                .client
                .post(format!("{}/Action/login", self.base))
                .json(&body)
                .send()
                .await
                .map_err(|_| "iKuai Web authentication communication failed".to_string())?;
            if !response.status().is_success() {
                return Err(format!("Web authentication HTTP {}", response.status()));
            }
            let result = decode(response).await?;
            if result.get("code").and_then(Value::as_i64) != Some(0) {
                return Err("iKuai Web authentication rejected".into());
            }
            self.authenticated.store(true, Ordering::SeqCst);
        }
        drop(permit);
        Ok(())
    }

    async fn request_call(
        &self,
        function: &str,
        action: &str,
        parameters: &Value,
    ) -> Result<Value, CallError> {
        let response = self
            .client
            .post(format!("{}/Action/call", self.base))
            .json(&json!({"func_name":function,"action":action,"param":parameters}))
            .send()
            .await
            .map_err(|_| CallError::Request("iKuai Web request communication failed".into()))?;
        if matches!(
            response.status(),
            reqwest::StatusCode::UNAUTHORIZED | reqwest::StatusCode::FORBIDDEN
        ) {
            self.authenticated.store(false, Ordering::SeqCst);
            return Err(CallError::SessionExpired);
        }
        if !response.status().is_success() {
            return Err(CallError::Request(format!(
                "iKuai Web HTTP {}",
                response.status()
            )));
        }
        let result = decode(response).await.map_err(CallError::Request)?;
        match result.get("code").and_then(Value::as_i64) {
            Some(0) => Ok(result.get("results").cloned().unwrap_or(Value::Null)),
            Some(1008 | 10014) => {
                self.authenticated.store(false, Ordering::SeqCst);
                Err(CallError::SessionExpired)
            }
            _ => Err(CallError::Request(format!(
                "iKuai rejected the {function} {action} request"
            ))),
        }
    }

    async fn read_call(&self, function: &str, parameters: &Value) -> Result<Value, String> {
        for attempt in 0..2 {
            self.login().await?;
            match self.request_call(function, "show", parameters).await {
                Ok(value) => return Ok(value),
                Err(CallError::SessionExpired) if attempt == 0 => {}
                Err(error) => return Err(error.message()),
            }
        }
        Err("iKuai Web authentication could not be renewed".into())
    }

    async fn edit_call(
        &self,
        function: &str,
        action: &str,
        parameters: &Value,
    ) -> Result<(), String> {
        self.login().await?;
        self.request_call(function, action, parameters)
            .await
            .map(|_| ())
            .map_err(CallError::message)
    }

    async fn interface_snapshot(&self) -> Result<Value, String> {
        let result = self
            .read_call("lan", &json!({"TYPE":"ether_info,netinfo,snapshoot"}))
            .await?;
        let wan_rows = result
            .get("snapshoot_wan")
            .and_then(Value::as_array)
            .ok_or_else(|| "iKuai Web WAN snapshot is missing".to_string())?;
        let lan_rows = result
            .get("snapshoot_lan")
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or_default();
        if wan_rows.len() + lan_rows.len() > 128 {
            return Err("iKuai interface snapshot exceeds its size limit".into());
        }
        let mut data = Vec::new();
        for (kind, rows) in [("wan", wan_rows.as_slice()), ("lan", lan_rows)] {
            for row in rows {
                let name = row
                    .get("interface")
                    .and_then(Value::as_str)
                    .filter(|name| {
                        !name.is_empty()
                            && name.len() <= 64
                            && name
                                .bytes()
                                .all(|byte| byte.is_ascii_alphanumeric() || b"_-.".contains(&byte))
                    })
                    .ok_or_else(|| "iKuai Web interface name is invalid".to_string())?;
                let default_route = if kind == "wan" {
                    row.get("default_route")
                        .and_then(Value::as_i64)
                        .filter(|value| [0, 1].contains(value))
                        .ok_or_else(|| "iKuai Web WAN default-route flag is invalid".to_string())?
                } else {
                    0
                };
                let comment = row
                    .get("comment")
                    .and_then(Value::as_str)
                    .filter(|text| text.len() <= 512 && !text.chars().any(char::is_control))
                    .unwrap_or_default();
                data.push(json!({"name":name,"kind":kind,"default_route":default_route,"comment":comment}));
            }
        }
        Ok(json!({"total":data.len(),"data":data}))
    }

    async fn wan_information(&self) -> Result<Value, String> {
        let snapshot = self.interface_snapshot().await?;
        let interfaces: Vec<String> = snapshot
            .get("data")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter(|row| row.get("kind").and_then(Value::as_str) == Some("wan"))
            .filter_map(|row| row.get("name").and_then(Value::as_str).map(str::to_string))
            .collect();
        let data: Vec<Value> = stream::iter(interfaces)
            .map(|interface| async move {
                let result = self
                    .read_call(
                        "homepage",
                        &json!({"TYPE":"wan_stat","ifname":interface,"interface":interface}),
                    )
                    .await
                    .ok()?;
                let row = result.get("wan_stat")?;
                if row.get("interface").and_then(Value::as_str) != Some(interface.as_str()) {
                    return None;
                }
                let code = row.get("isp")?.as_str()?.trim();
                if code.is_empty()
                    || code.len() > 64
                    || code.chars().any(char::is_control)
                    || ["0", "unknown", "none", "null"]
                        .iter()
                        .any(|value| code.eq_ignore_ascii_case(value))
                {
                    return None;
                }
                let name = match code {
                    "CTCC" => "中国电信",
                    "CUCC" => "中国联通",
                    "CMCC" => "中国移动",
                    _ => code,
                };
                Some(json!({"interface":interface,"display_name":name}))
            })
            .buffer_unordered(4)
            .filter_map(|row| async move { row })
            .collect()
            .await;
        Ok(json!({"total":data.len(),"data":data}))
    }

    pub async fn get(&self, path: &str) -> Result<Value, String> {
        let route = path.split('?').next().unwrap_or_default();
        match route {
            "/monitoring/system" => self.read_call("homepage", &json!({"TYPE":"sysstat"})).await,
            "/monitoring/system-samples" => self.read_call("monitor_system", &json!({})).await,
            "/monitoring/wan-information" => self.wan_information().await,
            "/internal/interfaces" => self.interface_snapshot().await,
            "/monitoring/interfaces-status" => {
                self.read_call("monitor_iface", &json!({"TYPE":"iface_check,iface_stream"}))
                    .await
            }
            "/interfaces/wan-config" => {
                let result = self
                    .read_call("lan", &json!({"TYPE":"ether_info,netinfo,snapshoot"}))
                    .await?;
                let rows = result
                    .get("snapshoot_wan")
                    .and_then(Value::as_array)
                    .ok_or_else(|| "iKuai Web WAN snapshot is missing".to_string())?;
                let data = rows
                    .iter()
                    .map(|row| {
                        let name = row
                            .get("interface")
                            .and_then(Value::as_str)
                            .ok_or_else(|| "iKuai Web WAN interface is missing".to_string())?;
                        let default = row
                            .get("default_route")
                            .and_then(Value::as_i64)
                            .filter(|value| [0, 1].contains(value))
                            .ok_or_else(|| {
                                "iKuai Web WAN default-route flag is invalid".to_string()
                            })?;
                        Ok(json!({"name":name,"default_route":default}))
                    })
                    .collect::<Result<Vec<_>, String>>()?;
                Ok(json!({"total":data.len(),"data":data}))
            }
            "/routing/five-tuple-rules"
            | "/routing/load-balance-rules"
            | "/routing/domain-rules" => {
                let function = match route {
                    "/routing/five-tuple-rules" => "stream_ipport",
                    "/routing/load-balance-rules" => "lb_pcc",
                    _ => "stream_domain",
                };
                let result = self
                    .read_call(function, &json!({"TYPE":"data,total","limit":"0,1000"}))
                    .await?;
                collection(&result, "data", "total")
            }
            "/internal/ipv6/capability" => {
                self.read_call("ipv6", &json!({"TYPE":"multi_support,bind_status"}))
                    .await
            }
            "/internal/ipv6/wan" | "/internal/ipv6/lan" => {
                let wan = route == "/internal/ipv6/wan";
                let result = self
                    .read_call(
                        "ipv6",
                        &json!({"TYPE":if wan {"data,total"} else {"lan_data,lan_total"}}),
                    )
                    .await?;
                collection(
                    &result,
                    if wan { "data" } else { "lan_data" },
                    if wan { "total" } else { "lan_total" },
                )
            }
            _ => Err("Unsupported iKuai Web resource".into()),
        }
    }

    pub async fn put(&self, resource: Resource, desired: &Value) -> Result<(), String> {
        let collection = self.get(resource.path()).await?;
        let row = collection
            .get("data")
            .and_then(Value::as_array)
            .and_then(|rows| rows.iter().find(|row| row.get("id") == desired.get("id")))
            .ok_or_else(|| "iKuai write id is absent".to_string())?;
        let mut expected = policy::normalized(resource, row)?;
        let field = resource.mutation_field();
        expected[field] = desired
            .get(field)
            .cloned()
            .ok_or_else(|| "iKuai changed field is absent".to_string())?;
        if expected != *desired {
            return Err("iKuai configuration changed before Web write".into());
        }
        let mut payload = if resource == Resource::DomainRules {
            policy::full_domain_payload(row, desired)?
        } else {
            desired.clone()
        };
        let (function, action) = match resource {
            Resource::FiveTupleRules => ("stream_ipport", "edit"),
            Resource::LoadBalanceRules => ("lb_pcc", "edit"),
            Resource::DomainRules | Resource::DomainRuleState => ("stream_domain", "edit"),
            Resource::Ipv6Wan | Resource::Ipv6Lan => {
                if row.get("internet").and_then(Value::as_str) != Some("dhcp") {
                    return Err("Only DHCP IPv6 profiles are supported".into());
                }
                if resource == Resource::Ipv6Wan {
                    for key in [
                        "ipv6_addr",
                        "ipv6_gateway",
                        "dhcp6_ip_addr",
                        "dhcp6_ip_gateway",
                        "dhcp6_prefix1",
                        "dhcp6_dns1",
                        "dhcp6_dns2",
                        "link_addr",
                    ] {
                        payload[key] = json!("");
                    }
                    ("ipv6", "wan_edit")
                } else {
                    payload["ipv6_addr"] = json!("");
                    payload["linkaddr"] = row.get("linkaddr").cloned().unwrap_or(json!(""));
                    ("ipv6", "edit")
                }
            }
        };
        self.edit_call(function, action, &payload).await
    }
}

async fn decode(mut response: reqwest::Response) -> Result<Value, String> {
    if response
        .content_length()
        .is_some_and(|length| length > MAX_RESPONSE_BYTES as u64)
    {
        return Err("iKuai Web response exceeds its size limit".into());
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| "iKuai Web response could not be read".to_string())?
    {
        if chunk.len() > MAX_RESPONSE_BYTES.saturating_sub(bytes.len()) {
            return Err("iKuai Web response exceeds its size limit".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&bytes).map_err(|_| "iKuai Web returned an invalid JSON response".into())
}

fn collection(result: &Value, data_key: &str, total_key: &str) -> Result<Value, String> {
    let data = result
        .get(data_key)
        .and_then(Value::as_array)
        .ok_or_else(|| "iKuai Web collection data is missing".to_string())?;
    let total = result
        .get(total_key)
        .and_then(Value::as_u64)
        .ok_or_else(|| "iKuai Web collection total is missing".to_string())?;
    Ok(json!({"data":data,"total":total}))
}
