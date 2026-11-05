use crate::{
    api::RouterApi,
    auth::{self, AuthState},
    config::Config,
    discovery,
    events::Events,
    journal::Store,
    logging::{self, LogStore},
    manual_selection::{self, SelectionError, SelectionMode, SelectionStore},
    policy::{self, Plan},
    settings,
    state::{self, Health, WanState},
};
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, State},
    http::StatusCode,
    middleware,
    routing::get,
};
use serde::Serialize;
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::sync::{RwLock, watch};
use tokio_util::sync::CancellationToken;
use tower_http::services::ServeDir;

const SETUP_REQUIRED: &str = "iKuai account credentials are not configured; fill ikuai.username and ikuai.password in config.yml, then restart the service";

#[derive(Clone, Serialize)]
pub struct Status {
    pub state: WanState,
    pub primary: Health,
    pub backup: Health,
    pub interface_addresses: BTreeMap<String, String>,
    pub interface_health: BTreeMap<String, Health>,
    pub apply_enabled: bool,
    pub ipv6_capability: String,
    pub ipv6_error: String,
    pub plan: Plan,
    pub last_error: String,
}

pub struct Runtime {
    pub api: Arc<dyn RouterApi>,
    pub config: Config,
    pub store: Store,
    pub events: Events,
    pub logs: Arc<LogStore>,
    pub settings_gate: tokio::sync::Mutex<()>,
    cycle_gate: tokio::sync::Mutex<()>,
    ipv6_gate: tokio::sync::Mutex<()>,
    selection: SelectionStore,
    wan_sampled: AtomicBool,
    ipv6_sampled: AtomicBool,
    ipv6_addresses_stale: AtomicBool,
    restart_request: tokio::sync::Notify,
    auth: Arc<AuthState>,
    ipv6_store: Option<Store>,
    egress_changes: watch::Sender<crate::state::Egress>,
    state: RwLock<WanState>,
    status: RwLock<Status>,
    ipv6_addresses: RwLock<Option<Vec<Value>>>,
    interface_names: RwLock<BTreeMap<String, String>>,
    ikuai_info: RwLock<Option<Value>>,
}

fn now_ms() -> u64 {
    match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(duration) => u64::try_from(duration.as_millis()).unwrap_or(u64::MAX),
        Err(_) => 0,
    }
}

impl Runtime {
    pub async fn new(config: Config, api: Arc<dyn RouterApi>) -> Result<Self, String> {
        let store = Store::open(&config.management.state_dir).await?;
        let logs = logging::initialize(&config);
        let events = Events::open(&config.notifications, &config.management.state_dir).await?;
        let auth =
            AuthState::with_state_dir(config.webui.clone(), &config.management.state_dir).await?;
        let state = store.state().await?;
        let selection = SelectionStore::open(&config.management.state_dir).await?;
        events
            .seed_active(if state.active == crate::state::Egress::Primary {
                "primary"
            } else {
                "backup"
            })
            .await?;
        let ipv6_store = if config.wan.ipv6_enabled {
            Some(Store::open(&config.management.state_dir.join("ipv6")).await?)
        } else {
            None
        };
        let (egress_changes, _) = watch::channel(state.active);
        let setup_required = config.wan.enabled && !api.connection_configured();
        let status = Status {
            state: state.clone(),
            primary: Health::Unknown,
            backup: Health::Unknown,
            interface_addresses: BTreeMap::new(),
            interface_health: BTreeMap::new(),
            apply_enabled: config.management.apply && !setup_required,
            ipv6_capability: if setup_required {
                "awaiting_configuration"
            } else {
                "checking"
            }
            .into(),
            ipv6_error: String::new(),
            plan: Plan::default(),
            last_error: if setup_required {
                SETUP_REQUIRED.into()
            } else {
                String::new()
            },
        };
        Ok(Self {
            api,
            config,
            store,
            events,
            logs,
            settings_gate: tokio::sync::Mutex::new(()),
            cycle_gate: tokio::sync::Mutex::new(()),
            ipv6_gate: tokio::sync::Mutex::new(()),
            selection,
            wan_sampled: AtomicBool::new(false),
            ipv6_sampled: AtomicBool::new(false),
            ipv6_addresses_stale: AtomicBool::new(false),
            restart_request: tokio::sync::Notify::new(),
            auth,
            ipv6_store,
            egress_changes,
            state: RwLock::new(state),
            status: RwLock::new(status),
            ipv6_addresses: RwLock::new(None),
            interface_names: RwLock::new(BTreeMap::new()),
            ikuai_info: RwLock::new(None),
        })
    }

    pub async fn cycle(&self) -> Result<(), String> {
        let _cycle = self.cycle_gate.lock().await;
        let result = self.cycle_parts(true).await;
        self.capture_ipv6_addresses().await;
        self.observe_events(result.is_err(), 0).await;
        result
    }

    pub async fn ipv4_cycle(&self) -> Result<(), String> {
        let _cycle = self.cycle_gate.lock().await;
        self.cycle_parts(false).await
    }

    async fn cycle_parts(&self, include_ipv6: bool) -> Result<(), String> {
        if !self.config.wan.enabled {
            return Ok(());
        }
        if self.setup_required() {
            return Err(SETUP_REQUIRED.into());
        }
        if self.config.management.apply {
            self.store
                .recover(self.api.as_ref(), &self.config.wan)
                .await?;
        }
        let monitor = self.api.get("/monitoring/interfaces-status").await?;
        if monitor
            .get("iface_check")
            .and_then(Value::as_array)
            .is_none()
        {
            return Err("爱快未返回线路检测结果，暂时无法判断线路状态。".into());
        }
        let primary = state::health(&monitor, &self.config.wan.primary);
        let backup = state::health(&monitor, &self.config.wan.backup);
        if primary != Health::Unknown || backup != Health::Unknown {
            self.wan_sampled.store(true, Ordering::SeqCst);
        }
        let interface_addresses: BTreeMap<String, String> = monitor
            .get("iface_check")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|row| {
                Some((
                    row.get("interface")?.as_str()?.to_string(),
                    row.get("ip_addr")?.as_str()?.to_string(),
                ))
            })
            .collect();
        let interface_health: BTreeMap<String, Health> = monitor
            .get("iface_check")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|row| {
                let interface = row.get("interface")?.as_str()?;
                Some((interface.to_string(), state::health(&monitor, interface)))
            })
            .collect();
        let now = now_ms();
        let automatic_target =
            self.state
                .write()
                .await
                .observe(primary, backup, &self.config.wan, now);
        let target = self.selection.mode().target().unwrap_or(automatic_target);
        let mut physical_config = self.config.wan.clone();
        physical_config.ipv6_enabled = false;
        let mut plan = policy::build(self.api.as_ref(), &physical_config, target).await?;
        if physical_config.rule_pairs.is_empty() && physical_config.domain_rules.is_empty() {
            plan.blockers.push("No managed IPv4 rules are configured; add actual primary and backup rule identifiers before enabling automatic failover".into());
        }
        if !self.config.management.apply && self.store.has_pending().await? {
            plan.blockers.push(
                "Unfinished IPv4 journal exists; dry-run will not mutate rules to recover it"
                    .into(),
            );
        }
        if (target == crate::state::Egress::Primary && primary != Health::Up)
            || (target == crate::state::Egress::Backup && backup != Health::Up)
        {
            plan.blockers
                .push("Selected egress is not healthy; current policies are preserved".into());
        }
        let last_error = plan.blockers.join("; ");
        let observed_state = self.state.read().await.clone();
        let previous_egress = observed_state.active;
        let previous_status = self.status.read().await.clone();
        let mut display_plan = plan.clone();
        if !include_ipv6 && self.config.wan.ipv6_enabled {
            display_plan.ipv6_capability = previous_status.ipv6_capability.clone();
            display_plan.ipv6_error = previous_status.ipv6_error.clone();
            display_plan.ipv6_evidence = previous_status.plan.ipv6_evidence;
            display_plan.changes.extend(
                previous_status
                    .plan
                    .changes
                    .into_iter()
                    .filter(|change| change.resource.is_ipv6()),
            );
        }
        *self.status.write().await = Status {
            state: observed_state,
            primary,
            backup,
            interface_addresses: interface_addresses.clone(),
            interface_health: interface_health.clone(),
            apply_enabled: self.config.management.apply,
            ipv6_capability: previous_status.ipv6_capability,
            ipv6_error: previous_status.ipv6_error,
            plan: display_plan,
            last_error: last_error.clone(),
        };
        if self.config.management.apply && plan.blockers.is_empty() {
            let changed_count = plan.changes.len();
            if !plan.changes.is_empty() {
                if let Err(error) = self
                    .store
                    .apply(self.api.as_ref(), &plan, &physical_config, now)
                    .await
                {
                    let saved = {
                        let mut state = self.state.write().await;
                        state.last_transition_ms = now_ms();
                        state.failed = 0;
                        state.recovered = 0;
                        state.clone()
                    };
                    self.store.save_state(&saved).await?;
                    self.status.write().await.last_error = error.clone();
                    return Err(error);
                }
            }
            let saved = {
                let mut state = self.state.write().await;
                state.commit(target, now);
                state.clone()
            };
            self.store.save_state(&saved).await?;
            let changed = saved.active != previous_egress;
            let active = saved.active;
            self.status.write().await.state = saved;
            if changed {
                self.egress_changes.send_replace(active);
            }
            if changed_count > 0 {
                self.notify(
                    "policy_synchronized",
                    "IPv4 分流策略已同步",
                    &format!("已写入并验证 {changed_count} 项 IPv4 出口策略。"),
                )
                .await;
            }
        }
        if include_ipv6 && self.config.wan.ipv6_enabled {
            let ipv6 = self
                .ipv6_cycle(
                    target,
                    self.config.management.apply && plan.blockers.is_empty(),
                )
                .await;
            plan.changes.extend(ipv6.changes);
            plan.ipv6_capability = ipv6.ipv6_capability;
            plan.ipv6_evidence = ipv6.ipv6_evidence;
            plan.ipv6_error = ipv6.ipv6_error;
            self.ipv6_sampled.store(true, Ordering::SeqCst);
        }
        let state = self.state.read().await.clone();
        let mut status = self.status.write().await;
        if !include_ipv6 && self.config.wan.ipv6_enabled {
            if state.active == previous_egress {
                plan.ipv6_capability = status.ipv6_capability.clone();
                plan.ipv6_error = status.ipv6_error.clone();
                plan.ipv6_evidence = status.plan.ipv6_evidence.clone();
                plan.changes.extend(
                    status
                        .plan
                        .changes
                        .iter()
                        .filter(|change| change.resource.is_ipv6())
                        .cloned(),
                );
            } else {
                plan.ipv6_capability = "checking".into();
            }
        }
        *status = Status {
            state,
            primary,
            backup,
            interface_addresses,
            interface_health,
            apply_enabled: self.config.management.apply,
            ipv6_capability: plan.ipv6_capability.clone(),
            ipv6_error: plan.ipv6_error.clone(),
            plan,
            last_error,
        };
        Ok(())
    }

    async fn ipv6_cycle(&self, target: crate::state::Egress, apply: bool) -> Plan {
        // Serialize the separate IPv6 journal without blocking IPv4 while PD settles.
        let _ipv6 = self.ipv6_gate.lock().await;
        let mut plan = Plan {
            target,
            ipv6_capability: "checking".into(),
            ..Plan::default()
        };
        let Some(store) = &self.ipv6_store else {
            plan.ipv6_capability = "disabled".into();
            return plan;
        };
        let recovery = if apply {
            store.recover(self.api.as_ref(), &self.config.wan).await
        } else {
            match store.has_pending().await {
                Ok(false) => Ok(()),
                Ok(true) => {
                    Err("Unfinished IPv6 journal exists; dry-run will not mutate it".into())
                }
                Err(error) => Err(error),
            }
        };
        if let Err(error) = recovery {
            plan.ipv6_error = error;
        } else if let Err(error) =
            crate::ipv6::extend_plan(self.api.as_ref(), &self.config.wan, &mut plan).await
        {
            plan.ipv6_error = error;
        } else if plan.changes.is_empty() {
            if plan
                .ipv6_evidence
                .as_ref()
                .is_some_and(crate::ipv6::Evidence::ready)
            {
                plan.ipv6_capability = "operational_verified".into();
            } else {
                plan.ipv6_error = "IPv6 WAN address, delegated prefix, or LAN prefix is unavailable on the selected egress".into();
            }
        } else if apply {
            match store
                .apply(self.api.as_ref(), &plan, &self.config.wan, now_ms())
                .await
            {
                Ok(()) => {
                    plan.ipv6_evidence = Some(crate::ipv6::Evidence {
                        wan_address_ready: true,
                        delegated_prefix_ready: true,
                        lan_prefix_ready: true,
                    });
                    plan.ipv6_capability = "operational_verified".into();
                    self.capture_ipv6_addresses().await;
                    self.notify(
                        "policy_synchronized",
                        "IPv6 策略已同步",
                        &format!(
                            "已写入并验证 {} 项 IPv6 WAN/PD/LAN 策略。",
                            plan.changes.len()
                        ),
                    )
                    .await;
                }
                Err(error) => {
                    plan.ipv6_error = error;
                    let mut observed = Plan {
                        target,
                        ..Plan::default()
                    };
                    if crate::ipv6::extend_plan(self.api.as_ref(), &self.config.wan, &mut observed)
                        .await
                        .is_ok()
                    {
                        plan.ipv6_evidence = observed.ipv6_evidence;
                    }
                }
            }
        }
        if !plan.ipv6_error.is_empty() {
            plan.ipv6_capability = "degraded".into();
        }
        plan
    }

    pub async fn snapshot(&self) -> Value {
        let status = self.status.read().await.clone();
        let interfaces: Vec<Value> = status
            .interface_addresses
            .iter()
            .map(|(interface, ip_addr)| json!({"interface":interface,"ip_addr":ip_addr}))
            .collect();
        let mut snapshot = json!({ "wan": status, "wan_health": status.interface_health, "interfaces": interfaces,
            "ikuai_info": self.ikuai_info.read().await.clone(),
            "manual_mode": self.selection.mode(),
            "initializing": !self.wan_sampled.load(Ordering::SeqCst) || (self.config.wan.ipv6_enabled && !self.ipv6_sampled.load(Ordering::SeqCst)),
            "ipv6_addresses_stale": self.ipv6_addresses_stale.load(Ordering::SeqCst),
            "wan_enabled": self.config.wan.enabled,
            "setup_required": self.setup_required(),
            "interface_names": self.interface_names.read().await.clone(),
            "primary_interface": self.config.wan.primary,
            "backup_interface": self.config.wan.backup,
            "preserved_default": self.config.wan.preserved_default,
            "notifications": self.events.status().await });
        if let Some(addresses) = self.ipv6_addresses.read().await.as_ref() {
            snapshot["ipv6_addresses"] = json!(addresses);
        }
        snapshot
    }

    fn setup_required(&self) -> bool {
        self.config.wan.enabled && !self.api.connection_configured()
    }

    pub async fn wan_selection(&self) -> Value {
        json!({"mode":self.selection.mode(),"active":self.state.read().await.active,
            "restart_resets":self.selection.restart_resets(),
            "enabled":self.config.wan.enabled && self.config.management.apply && !self.setup_required(),
            "primary_interface":self.config.wan.primary,"backup_interface":self.config.wan.backup})
    }

    pub(crate) async fn select_wan(&self, mode: SelectionMode) -> Result<(), SelectionError> {
        let _cycle = self.cycle_gate.try_lock().map_err(|_| {
            SelectionError::new(
                StatusCode::CONFLICT,
                "wan_selection_busy",
                "服务正在检查或应用线路策略，请稍后重试。",
            )
        })?;
        if mode != SelectionMode::Auto
            && (!self.config.wan.enabled || !self.config.management.apply || self.setup_required())
        {
            return Err(SelectionError::new(
                StatusCode::CONFLICT,
                "wan_selection_unavailable",
                "请先连接爱快，并开启线路监测和自动应用后再选择出口。",
            ));
        }
        let previous_mode = self.selection.mode();
        if let Some(target) = mode.target() {
            let monitor = self
                .api
                .get("/monitoring/interfaces-status")
                .await
                .map_err(|error| {
                    logging::error(&format!("[出口选择:检查失败] {error}"));
                    SelectionError::new(
                        StatusCode::SERVICE_UNAVAILABLE,
                        "wan_selection_failed",
                        "无法检查爱快线路状态，尚未切换出口。",
                    )
                })?;
            let interface = if target == state::Egress::Primary {
                &self.config.wan.primary
            } else {
                &self.config.wan.backup
            };
            if state::health(&monitor, interface) != Health::Up {
                return Err(SelectionError::new(
                    StatusCode::UNPROCESSABLE_ENTITY,
                    "wan_selection_blocked",
                    "所选线路尚未检测为正常，当前出口已保留。",
                ));
            }
            let mut physical_config = self.config.wan.clone();
            physical_config.ipv6_enabled = false;
            let plan = policy::build(self.api.as_ref(), &physical_config, target)
                .await
                .map_err(|error| {
                    logging::error(&format!("[出口选择:规则检查失败] {error}"));
                    SelectionError::new(
                        StatusCode::SERVICE_UNAVAILABLE,
                        "wan_selection_failed",
                        "无法检查所选线路的分流规则，尚未切换出口。",
                    )
                })?;
            if physical_config.rule_pairs.is_empty() && physical_config.domain_rules.is_empty() {
                return Err(SelectionError::new(
                    StatusCode::UNPROCESSABLE_ENTITY,
                    "wan_selection_blocked",
                    "请先配置主备线路对应的分流规则。",
                ));
            }
            if !plan.blockers.is_empty() {
                let mut error = SelectionError::new(
                    StatusCode::UNPROCESSABLE_ENTITY,
                    "wan_selection_blocked",
                    "分流规则检查未通过，当前出口已保留。",
                );
                error.blockers = plan.blockers;
                return Err(error);
            }
            if self.store.has_pending().await.map_err(|_| {
                SelectionError::new(
                    StatusCode::SERVICE_UNAVAILABLE,
                    "wan_selection_failed",
                    "无法检查上次切换记录，尚未切换出口。",
                )
            })? {
                return Err(SelectionError::new(
                    StatusCode::CONFLICT,
                    "wan_selection_blocked",
                    "上次切换正在恢复，请等待本轮检查完成后重试。",
                ));
            }
        }
        self.selection.save(mode).await.map_err(|error| {
            logging::error(&format!("[出口选择:保存失败] {error}"));
            SelectionError::new(
                StatusCode::SERVICE_UNAVAILABLE,
                "wan_selection_failed",
                "无法保存出口选择模式，尚未切换线路。",
            )
        })?;
        if mode != SelectionMode::Auto {
            let result = self.cycle_parts(false).await;
            let blocked = !self.status.read().await.plan.blockers.is_empty();
            if result.is_err() || blocked {
                let mut restored = true;
                if let Err(error) = self.selection.save(previous_mode).await {
                    logging::error(&format!("[出口选择:模式恢复失败] {error}"));
                    self.selection.restore_in_memory(previous_mode);
                    restored = false;
                }
                if let Err(error) = result {
                    logging::error(&format!("[出口选择:应用失败] {error}"));
                }
                return Err(SelectionError::new(
                    StatusCode::SERVICE_UNAVAILABLE,
                    "wan_selection_failed",
                    if restored {
                        "本次切换未完成，已恢复原选择模式，请查看运行日志并刷新线路状态。"
                    } else {
                        "本次切换未完成，原模式仅在当前运行中恢复；重启后可能使用本次选择，请检查状态目录写入权限。"
                    },
                ));
            }
        }
        let label = match mode {
            SelectionMode::Auto => "自动选择主备线路",
            SelectionMode::Primary => "保持使用主线路",
            SelectionMode::Backup => "保持使用备用线路",
        };
        self.notify("egress_selection_changed", "出口选择已更新", label)
            .await;
        self.observe_events(false, 0).await;
        Ok(())
    }

    pub fn request_restart(&self) {
        self.restart_request.notify_one();
    }

    pub async fn notify(&self, kind: &str, title: &str, text: &str) {
        let snapshot = self.snapshot().await;
        let context =
            crate::notification::network_context::NetworkContext::from_snapshot(&snapshot);
        match tokio::time::timeout(
            Duration::from_secs(2),
            self.events.emit_with_context(kind, title, text, context),
        )
        .await
        {
            Ok(Ok(())) => {}
            _ => logging::warn("[通知:记录失败] Notification event could not be persisted"),
        }
    }

    async fn observe_events(&self, failed: bool, changes: usize) {
        let snapshot = self.snapshot().await;
        match tokio::time::timeout(
            Duration::from_secs(2),
            self.events.observe(&snapshot, failed, changes),
        )
        .await
        {
            Ok(Ok(())) => {}
            _ => logging::warn("[通知:观察失败] Notification observation could not be persisted"),
        }
    }

    async fn capture_ipv6_addresses(&self) {
        if !self.config.wan.ipv6_enabled || !self.api.ipv6_supported() {
            return;
        }
        self.ipv6_addresses_stale.store(true, Ordering::SeqCst);
        if let Ok(value) = self.api.get("/internal/ipv6/wan").await {
            if let Some(rows) = value.get("data").and_then(Value::as_array) {
                let safe: Vec<Value> = rows.iter().map(|row| json!({"interface":row.get("interface"),
                    "dhcp6_ip_addr":row.get("dhcp6_ip_addr"),"dhcp6_prefix1":row.get("dhcp6_prefix1")})).collect();
                *self.ipv6_addresses.write().await = Some(safe);
                self.ipv6_addresses_stale.store(false, Ordering::SeqCst);
            }
        }
    }

    async fn wan_loop(&self, cancel: CancellationToken) -> Result<(), String> {
        if !self.config.wan.enabled || self.setup_required() {
            cancel.cancelled().await;
            return Ok(());
        }
        loop {
            if cancel.is_cancelled() {
                return Ok(());
            }
            let cycle_guard = tokio::select! {
                () = cancel.cancelled() => return Ok(()),
                guard = self.cycle_gate.lock() => guard,
            };
            match tokio::time::timeout(Duration::from_secs(120), self.cycle_parts(false)).await {
                Ok(Ok(())) => {}
                result => {
                    let error = match result {
                        Ok(Err(error)) => error,
                        _ => "WAN cycle exceeded its timeout; journal recovery is required".into(),
                    };
                    let error = if !self.wan_sampled.load(Ordering::SeqCst) {
                        format!("首次线路检查失败，暂时无法判断线路状态：{error}")
                    } else {
                        error
                    };
                    self.status.write().await.last_error = error.clone();
                    logging::error(&format!("[WAN:更新失败] {error}"));
                }
            }
            drop(cycle_guard);
            let failed = !self.status.read().await.last_error.is_empty();
            self.observe_events(failed, 0).await;
            tokio::select! {
                () = cancel.cancelled() => return Ok(()),
                () = tokio::time::sleep(self.config.wan.interval()) => {},
            }
        }
    }

    async fn ikuai_info_loop(&self, cancel: CancellationToken) -> Result<(), String> {
        loop {
            if self.api.connection_configured() {
                let update = async {
                    let value = self.api.get("/monitoring/system").await.ok()?;
                    let mut info = crate::ikuai_info::normalize(&value)?;
                    if info.get("cpu_percent").is_none() || info.get("memory_percent").is_none() {
                        if let Ok(samples) = self.api.get("/monitoring/system-samples").await {
                            crate::ikuai_info::add_monitor_sample(&mut info, &samples);
                        }
                    }
                    info["stale"] = json!(false);
                    info["updated_at_ms"] = json!(now_ms());
                    Some(info)
                };
                tokio::select! {
                    () = cancel.cancelled() => return Ok(()),
                    result = tokio::time::timeout(Duration::from_secs(15), update) => {
                        let mut saved = self.ikuai_info.write().await;
                        if let Ok(Some(info)) = result { *saved = Some(info); }
                        else if let Some(info) = saved.as_mut() { info["stale"] = json!(true); }
                    }
                }
            }
            tokio::select! {
                () = cancel.cancelled() => return Ok(()),
                () = tokio::time::sleep(Duration::from_secs(30)) => {},
            }
        }
    }

    async fn interface_names_loop(&self, cancel: CancellationToken) -> Result<(), String> {
        if !self.config.wan.enabled || self.setup_required() {
            cancel.cancelled().await;
            return Ok(());
        }
        loop {
            if cancel.is_cancelled() {
                return Ok(());
            }
            if let Ok(value) = self.api.get("/monitoring/wan-information").await {
                let names = value
                    .get("data")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(|row| {
                        let interface = row.get("interface")?.as_str()?;
                        let name = row.get("display_name")?.as_str()?;
                        (!name.is_empty()).then(|| (interface.to_string(), name.to_string()))
                    })
                    .collect();
                *self.interface_names.write().await = names;
            }
            tokio::select! {
                () = cancel.cancelled() => return Ok(()),
                () = tokio::time::sleep(Duration::from_secs(60)) => {},
            }
        }
    }

    async fn ipv6_loop(&self, cancel: CancellationToken) -> Result<(), String> {
        if !self.config.wan.enabled || !self.config.wan.ipv6_enabled || self.setup_required() {
            cancel.cancelled().await;
            return Ok(());
        }
        let mut egress = self.egress_changes.subscribe();
        loop {
            if cancel.is_cancelled() {
                return Ok(());
            }
            let _ = *egress.borrow_and_update();
            if !self.wan_sampled.load(Ordering::SeqCst) {
                tokio::select! {
                    () = cancel.cancelled() => return Ok(()),
                    () = tokio::time::sleep(self.config.wan.interval()) => {},
                }
                continue;
            }
            let target = self.state.read().await.active;
            let apply = {
                let status = self.status.read().await;
                let selected = if target == crate::state::Egress::Primary {
                    status.primary
                } else {
                    status.backup
                };
                self.config.management.apply
                    && selected == Health::Up
                    && status.last_error.is_empty()
            };
            let plan = match tokio::time::timeout(
                Duration::from_secs(120),
                self.ipv6_cycle(target, apply),
            )
            .await
            {
                Ok(plan) => plan,
                Err(_) => Plan {
                    target,
                    ipv6_capability: "degraded".into(),
                    ipv6_error:
                        "IPv6 cycle exceeded its timeout; its separate journal will be recovered"
                            .into(),
                    ..Plan::default()
                },
            };
            if self.state.read().await.active != target {
                continue;
            }
            {
                let mut status = self.status.write().await;
                if status.state.active == target {
                    status.ipv6_capability = plan.ipv6_capability.clone();
                    status.ipv6_error = plan.ipv6_error.clone();
                    status.plan.ipv6_capability = plan.ipv6_capability;
                    status.plan.ipv6_error = plan.ipv6_error;
                    status.plan.ipv6_evidence = plan.ipv6_evidence;
                    status
                        .plan
                        .changes
                        .retain(|change| !change.resource.is_ipv6());
                    status.plan.changes.extend(plan.changes);
                }
            }
            self.capture_ipv6_addresses().await;
            self.ipv6_sampled.store(true, Ordering::SeqCst);
            let failed = !self.status.read().await.last_error.is_empty();
            self.observe_events(failed, 0).await;
            tokio::select! {
                () = cancel.cancelled() => return Ok(()),
                changed = egress.changed() => changed.map_err(|e| e.to_string())?,
                () = tokio::time::sleep(self.config.wan.interval()) => {},
            }
        }
    }

    async fn notification_loop(&self, cancel: CancellationToken) -> Result<(), String> {
        loop {
            match self.events.run(cancel.child_token()).await {
                Ok(()) => return Ok(()),
                Err(_) => {
                    logging::warn("[通知:后台失败] Notification worker will retry independently")
                }
            }
            tokio::select! {
                () = cancel.cancelled() => return Ok(()),
                () = tokio::time::sleep(Duration::from_secs(5)) => {},
            }
        }
    }

    pub async fn run(self: Arc<Self>, cancel: CancellationToken) -> Result<(), String> {
        let listener = tokio::net::TcpListener::bind(self.config.management.listen)
            .await
            .map_err(|e| e.to_string())?;
        let protected = Router::new()
            .route("/status", get(status))
            .route(
                "/wan/selection",
                get(manual_selection::get)
                    .post(manual_selection::set)
                    .layer(DefaultBodyLimit::max(1024)),
            )
            .route("/notifications", get(notifications))
            .route(
                "/notifications/templates/defaults",
                get(crate::notification::testing::defaults),
            )
            .route(
                "/notifications/webpush/key",
                get(crate::notification::webpush_settings::public_key)
                    .post(crate::notification::webpush_settings::initialize_key),
            )
            .route(
                "/notifications/webpush/subscribe",
                axum::routing::post(crate::notification::webpush_settings::subscribe)
                    .layer(DefaultBodyLimit::max(32 * 1024)),
            )
            .route(
                "/notifications/clawbot/status",
                get(crate::notification::clawbot_settings::status),
            )
            .route(
                "/notifications/clawbot/qrcode",
                axum::routing::post(crate::notification::clawbot_settings::qrcode),
            )
            .route(
                "/notifications/clawbot/poll",
                axum::routing::post(crate::notification::clawbot_settings::poll),
            )
            .route(
                "/notifications/clawbot/logout",
                axum::routing::post(crate::notification::clawbot_settings::logout),
            )
            .route(
                "/notifications/test",
                axum::routing::post(crate::notification::testing::send_test)
                    .layer(DefaultBodyLimit::max(2 * 1024 * 1024)),
            )
            .route(
                "/notifications/template/preview",
                axum::routing::post(crate::notification::testing::preview)
                    .layer(DefaultBodyLimit::max(64 * 1024)),
            )
            .route("/logs/files", get(logging::files))
            .route("/logs/tail", get(logging::tail))
            .route("/logs/download", get(logging::download))
            .route(
                "/settings/ikuai",
                get(settings::get)
                    .post(settings::save)
                    .layer(DefaultBodyLimit::max(16 * 1024)),
            )
            .route(
                "/settings/options",
                get(discovery::get)
                    .post(discovery::preview)
                    .layer(DefaultBodyLimit::max(16 * 1024)),
            )
            .route(
                "/settings/config",
                get(settings::get_config)
                    .post(settings::save_config)
                    .layer(DefaultBodyLimit::max(4 * 1024 * 1024)),
            )
            .route("/system/restart", axum::routing::post(settings::restart))
            .with_state(Arc::clone(&self))
            .route_layer(middleware::from_fn_with_state(
                Arc::clone(&self.auth),
                auth::protect,
            ));
        let app = Router::new()
            .route("/appearance/wallpaper", get(crate::appearance::wallpaper))
            .route(
                "/appearance/wallpaper/image",
                get(crate::appearance::wallpaper_image),
            )
            .route("/healthz", get(|| async { Json(json!({"status":"ok"})) }))
            .route("/readyz", get(ready))
            .with_state(Arc::clone(&self))
            .merge(protected)
            .merge(auth::routes(Arc::clone(&self.auth)))
            .fallback_service(ServeDir::new(&self.config.management.web_dir));
        let http =
            axum::serve(listener, app).with_graceful_shutdown(cancel.clone().cancelled_owned());
        let services = async {
            tokio::try_join!(
                self.wan_loop(cancel.child_token()),
                self.ipv6_loop(cancel.child_token()),
                self.interface_names_loop(cancel.child_token()),
                self.ikuai_info_loop(cancel.child_token()),
                self.notification_loop(cancel.child_token()),
                async { http.await.map_err(|e| e.to_string()) }
            )
        };
        tokio::pin!(services);
        let result = tokio::select! {
            result = &mut services => result,
            () = self.restart_request.notified() => {
                self.notify("service_restarting", "服务正在重启", "配置已保存，正在重新加载服务。").await;
                cancel.cancel();
                services.await
            }
        };
        cancel.cancel();
        result.map(|_| ())
    }
}

async fn status(State(runtime): State<Arc<Runtime>>) -> Json<Value> {
    Json(runtime.snapshot().await)
}
async fn notifications(State(runtime): State<Arc<Runtime>>) -> Json<Value> {
    Json(runtime.events.history().await)
}
async fn ready(State(runtime): State<Arc<Runtime>>) -> (StatusCode, Json<Value>) {
    let state = runtime.status.read().await;
    let selected = if state.state.active == crate::state::Egress::Primary {
        state.primary
    } else {
        state.backup
    };
    let ready = runtime.config.wan.enabled
        && !runtime.setup_required()
        && selected == Health::Up
        && state.last_error.is_empty()
        && state.plan.blockers.is_empty();
    (
        if ready {
            StatusCode::OK
        } else {
            StatusCode::SERVICE_UNAVAILABLE
        },
        Json(json!({ "ready": ready, "setup_required": runtime.setup_required() })),
    )
}
