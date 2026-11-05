use std::{
    collections::{BTreeMap, VecDeque},
    net::IpAddr,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio::{
    io::AsyncWriteExt,
    sync::{Mutex, Notify, Semaphore},
};
use tokio_util::sync::CancellationToken;

use crate::notification::{DeliveryResult, Event, NotificationConfig, Notifier};

const HISTORY_CAPACITY: usize = 200;
const MAX_ATTEMPTS: u8 = 5;

#[async_trait]
pub trait EventSender: Send + Sync {
    async fn send_to(&self, event: &Event, tags: &[String]) -> Vec<DeliveryResult>;
}

#[async_trait]
impl EventSender for Notifier {
    async fn send_to(&self, event: &Event, tags: &[String]) -> Vec<DeliveryResult> {
        Notifier::send_to(self, event, tags).await
    }
}

#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(default)]
struct Baseline {
    initialized: bool,
    health: BTreeMap<String, String>,
    ipv4: BTreeMap<String, String>,
    ipv6: BTreeMap<String, String>,
    prefixes: BTreeMap<String, String>,
    active: String,
    pending: bool,
    conflict: bool,
    cycle_failed: bool,
    ipv6_degraded: bool,
}

#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(default)]
struct Counters {
    queued: u64,
    suppressed: u64,
    filtered: u64,
    dropped: u64,
    delivered: u64,
    failed_attempts: u64,
    exhausted: u64,
    retries: u64,
}

#[derive(Clone, Deserialize, Serialize)]
struct Target {
    tag: String,
    attempts: u8,
    next_ms: u64,
    #[serde(skip)]
    in_flight: bool,
}

#[derive(Clone, Deserialize, Serialize)]
struct Pending {
    event: Event,
    targets: Vec<Target>,
}

#[derive(Clone, Deserialize, Serialize)]
struct History {
    event: Event,
    deliveries: Vec<DeliveryResult>,
    pending_tags: Vec<String>,
    status: String,
}

#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(default)]
struct Stored {
    baseline: Baseline,
    queue: VecDeque<Pending>,
    history: VecDeque<History>,
    cooldown: BTreeMap<String, u64>,
    counters: Counters,
    sequence: u64,
    network_context: crate::notification::network_context::NetworkContext,
    observed_network_context: Option<crate::notification::network_context::NetworkContext>,
}

pub struct Events {
    config: NotificationConfig,
    sender: Arc<dyn EventSender>,
    path: PathBuf,
    stored: Mutex<Stored>,
    write_gate: Semaphore,
    worker_gate: Semaphore,
    wake: Notify,
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|time| u64::try_from(time.as_millis()).ok())
        .unwrap_or(0)
}

fn safe_text(text: &str, limit: usize) -> String {
    let mut secret_next = false;
    text.split_whitespace()
        .map(|word| {
            let lower = word.to_ascii_lowercase();
            let secret = secret_next
                || lower.contains("://")
                || ["token=", "password=", "secret=", "passwd="]
                    .iter()
                    .any(|key| lower.contains(key));
            secret_next = lower == "bearer";
            if secret { "[redacted]" } else { word }
        })
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(limit)
        .collect()
}

fn event(kind: &str, title: &str, text: String) -> Event {
    Event {
        id: String::new(),
        kind: kind.into(),
        title: title.into(),
        text,
        timestamp_ms: 0,
        context: Default::default(),
        previous_context: None,
    }
}

fn kind_valid(kind: &str) -> bool {
    !kind.is_empty()
        && kind.len() <= 64
        && kind
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte == b'_')
}

impl Events {
    pub async fn open(config: &NotificationConfig, state_dir: &Path) -> Result<Self, String> {
        let sender = Notifier::new(config).map_err(|error| error.to_string())?;
        Self::open_with_sender(config, state_dir, Arc::new(sender)).await
    }

    pub async fn open_with_sender(
        config: &NotificationConfig,
        state_dir: &Path,
        sender: Arc<dyn EventSender>,
    ) -> Result<Self, String> {
        config.validate().map_err(|error| error.to_string())?;
        tokio::fs::create_dir_all(state_dir)
            .await
            .map_err(|error| error.to_string())?;
        let path = state_dir.join("notifications.json");
        let mut stored: Stored = match tokio::fs::read(&path).await {
            Ok(bytes) => match serde_json::from_slice(&bytes) {
                Ok(stored) => stored,
                Err(_) => {
                    let timestamp = SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .map_or(0, |time| time.as_nanos());
                    let backup = state_dir.join(format!("notifications.corrupt-{timestamp}.json"));
                    tokio::fs::rename(&path, backup).await.map_err(|_| {
                        "Corrupt notification state cannot be preserved".to_string()
                    })?;
                    crate::logging::warn(
                        "[通知:状态恢复] 损坏状态已保留，通知基线与队列重新初始化。",
                    );
                    Stored::default()
                }
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Stored::default(),
            Err(_) => return Err("Notification state cannot be read".into()),
        };
        while stored.history.len() > HISTORY_CAPACITY {
            stored.history.pop_front();
        }
        while stored.queue.len() > config.queue_capacity {
            if let Some(pending) = stored.queue.pop_front() {
                finish_history(&mut stored, &pending.event.id, Vec::new(), "dropped");
                stored.counters.dropped = stored.counters.dropped.saturating_add(1);
            }
        }
        stored.baseline = Baseline {
            active: stored.baseline.active,
            ..Baseline::default()
        };
        Ok(Self {
            config: config.clone(),
            sender,
            path,
            stored: Mutex::new(stored),
            write_gate: Semaphore::new(1),
            worker_gate: Semaphore::new(1),
            wake: Notify::new(),
        })
    }

    async fn mutate<T>(&self, change: impl FnOnce(&mut Stored) -> T) -> Result<T, String> {
        let permit = self
            .write_gate
            .acquire()
            .await
            .map_err(|_| "Notification state writer is unavailable".to_string())?;
        let (result, before, snapshot) = {
            let mut stored = self.stored.lock().await;
            let before = serde_json::to_vec(&*stored)
                .map_err(|_| "Notification state cannot be encoded".to_string())?;
            let result = change(&mut stored);
            (result, before, stored.clone())
        };
        let bytes = serde_json::to_vec(&snapshot)
            .map_err(|_| "Notification state cannot be encoded".to_string())?;
        if bytes == before {
            return Ok(result);
        }
        let temporary = self.path.with_extension("tmp");
        let mut file = tokio::fs::File::create(&temporary)
            .await
            .map_err(|_| "Notification state cannot be written".to_string())?;
        file.write_all(&bytes)
            .await
            .map_err(|_| "Notification state write failed".to_string())?;
        file.sync_all()
            .await
            .map_err(|_| "Notification state synchronization failed".to_string())?;
        drop(file);
        tokio::fs::rename(&temporary, &self.path)
            .await
            .map_err(|_| "Notification state replacement failed".to_string())?;
        drop(permit);
        Ok(result)
    }

    fn enqueue(&self, stored: &mut Stored, mut value: Event, now: u64) {
        let affected_interface = value.context.affected_interface.clone();
        if value.context.interface.is_empty() {
            value.context = stored.network_context.clone();
        }
        if crate::notification::network_context::transition_kind(&value.kind)
            && value.context.previous.is_none()
        {
            let previous = stored
                .observed_network_context
                .as_ref()
                .unwrap_or(&stored.network_context);
            value.context = value.context.with_transition(previous, &affected_interface);
        }
        if value.previous_context.is_none() {
            value.previous_context = value.context.previous.as_deref().cloned();
        }
        value.title = safe_text(&value.title, 160);
        value.text = safe_text(&value.text, 4096);
        let hash = format!(
            "{:x}",
            Sha256::digest(format!("{}\0{}\0{}", value.kind, value.title, value.text))
        );
        stored
            .cooldown
            .retain(|_, time| now.saturating_sub(*time) < self.config.cooldown_ms);
        if stored.cooldown.contains_key(&hash) {
            stored.counters.suppressed = stored.counters.suppressed.saturating_add(1);
            return;
        }
        if stored.cooldown.len() >= 1024 {
            if let Some(oldest) = stored
                .cooldown
                .iter()
                .min_by_key(|(_, time)| *time)
                .map(|(key, _)| key.clone())
            {
                stored.cooldown.remove(&oldest);
            }
        }
        stored.cooldown.insert(hash.clone(), now);
        stored.sequence = stored.sequence.saturating_add(1);
        value.id = format!("{now}-{}-{}", stored.sequence, &hash[..12]);
        value.timestamp_ms = now;
        let message = format!("[{}] {}：{}", value.kind, value.title, value.text);
        if value.kind.ends_with("failed") || value.kind == "policy_conflict" {
            crate::logging::error(&message);
        } else if value.kind.ends_with("degraded") || value.kind == "failover_pending" {
            crate::logging::warn(&message);
        } else {
            crate::logging::info(&message);
        }
        let selected = if self.config.events.is_empty() {
            !matches!(
                value.kind.as_str(),
                "service_started" | "service_restarting" | "service_stopped" | "service_failed"
            )
        } else {
            self.config.events.contains(&value.kind)
        };
        let tags: Vec<String> = if self.config.enabled && selected {
            self.config
                .channels
                .iter()
                .filter(|channel| {
                    channel.enabled
                        && (channel.switchs.is_empty() || channel.switchs.contains(&value.kind))
                })
                .map(|channel| channel.tag.clone())
                .collect()
        } else {
            Vec::new()
        };
        let status = if !selected {
            "filtered"
        } else if tags.is_empty() {
            "disabled"
        } else {
            "queued"
        };
        if !selected {
            stored.counters.filtered = stored.counters.filtered.saturating_add(1);
        }
        if !tags.is_empty() {
            if stored.queue.len() >= self.config.queue_capacity {
                if let Some(old) = stored.queue.pop_front() {
                    finish_history(stored, &old.event.id, Vec::new(), "dropped");
                }
                stored.counters.dropped = stored.counters.dropped.saturating_add(1);
            }
            stored.queue.push_back(Pending {
                event: value.clone(),
                targets: tags
                    .iter()
                    .map(|tag| Target {
                        tag: tag.clone(),
                        attempts: 0,
                        next_ms: now,
                        in_flight: false,
                    })
                    .collect(),
            });
            stored.counters.queued = stored.counters.queued.saturating_add(1);
        }
        stored.history.push_back(History {
            event: crate::notification::templates::render(
                &crate::notification::templates::select(&self.config.templates, &value.kind),
                &value,
            )
            .unwrap_or(value),
            deliveries: Vec::new(),
            pending_tags: tags,
            status: status.into(),
        });
        while stored.history.len() > HISTORY_CAPACITY {
            stored.history.pop_front();
        }
    }

    pub async fn emit(&self, kind: &str, title: &str, text: &str) -> Result<(), String> {
        if !kind_valid(kind) {
            return Err("Notification event kind is invalid".into());
        }
        self.mutate(|stored| self.enqueue(stored, event(kind, title, text.into()), now_ms()))
            .await?;
        self.wake.notify_one();
        Ok(())
    }

    pub async fn emit_with_context(
        &self,
        kind: &str,
        title: &str,
        text: &str,
        context: crate::notification::network_context::NetworkContext,
    ) -> Result<(), String> {
        if !kind_valid(kind) {
            return Err("Notification kind is invalid".into());
        }
        self.mutate(move |stored| {
            let previous = stored
                .observed_network_context
                .clone()
                .unwrap_or_else(|| stored.network_context.clone());
            let context = context.with_previous(&stored.network_context);
            stored.network_context = context.clone();
            let mut value = event(kind, title, text.into());
            value.context = if crate::notification::network_context::transition_kind(kind) {
                context.with_transition(&previous, "")
            } else {
                context
            };
            self.enqueue(stored, value, now_ms());
        })
        .await?;
        self.wake.notify_one();
        Ok(())
    }

    pub async fn seed_active(&self, active: &str) -> Result<(), String> {
        if !["primary", "backup"].contains(&active) {
            return Err("Initial egress is invalid".into());
        }
        self.mutate(|stored| {
            if stored.baseline.active.is_empty() || !stored.baseline.initialized {
                stored.baseline.active = active.into();
            }
        })
        .await
    }

    pub async fn set_network_context(
        &self,
        context: crate::notification::network_context::NetworkContext,
    ) {
        let mut stored = self.stored.lock().await;
        stored.network_context = context.with_previous(&stored.network_context);
    }

    pub async fn test_network_context(
        &self,
        context: crate::notification::network_context::NetworkContext,
        kind: &str,
    ) -> crate::notification::network_context::NetworkContext {
        let stored = self.stored.lock().await;
        let previous = stored
            .observed_network_context
            .as_ref()
            .unwrap_or(&stored.network_context);
        let context = context.with_previous(previous);
        if crate::notification::network_context::transition_kind(kind) {
            context.with_transition(previous, "")
        } else {
            context
        }
    }

    pub async fn observe(
        &self,
        snapshot: &Value,
        cycle_failed: bool,
        applied_changes: usize,
    ) -> Result<(), String> {
        if snapshot.get("initializing").and_then(Value::as_bool) == Some(true) {
            return Ok(());
        }
        self.mutate(|stored| {
            let previous = stored
                .observed_network_context
                .clone()
                .unwrap_or_else(|| stored.network_context.clone());
            let context =
                crate::notification::network_context::NetworkContext::from_snapshot(snapshot)
                    .with_previous(&stored.network_context);
            stored.network_context = context.clone();
            let before = stored.baseline.clone();
            let mut next = baseline(snapshot, &before);
            next.cycle_failed = cycle_failed;
            let mut changes = Vec::new();
            if before.initialized {
                compare_map(
                    &before.health,
                    &next.health,
                    "wan_health_changed",
                    "线路状态变化",
                    &mut changes,
                );
                compare_map(
                    &before.ipv4,
                    &next.ipv4,
                    "wan_ipv4_changed",
                    "WAN IPv4 地址变化",
                    &mut changes,
                );
                compare_map(
                    &before.ipv6,
                    &next.ipv6,
                    "wan_ipv6_changed",
                    "WAN IPv6 地址变化",
                    &mut changes,
                );
                compare_map(
                    &before.prefixes,
                    &next.prefixes,
                    "ipv6_pd_changed",
                    "IPv6 PD 前缀变化",
                    &mut changes,
                );
            }
            if !before.active.is_empty() && next.active != before.active && !next.active.is_empty()
            {
                let (kind, title) = if next.active == "primary" {
                    ("egress_recovered", "主线路出口恢复回切")
                } else {
                    ("egress_switched", "物理出口已切换")
                };
                changes.push(event(
                    kind,
                    title,
                    format!(
                        "物理出口由 {} 切换到 {}，保留的默认路径不变。",
                        before.active, next.active
                    ),
                ));
            }
            for (old, new, kind, title, body) in [
                (
                    before.pending,
                    next.pending,
                    "failover_pending",
                    "等待故障转移",
                    "当前线路正在满足故障阈值或等待策略应用。",
                ),
                (
                    before.conflict,
                    next.conflict,
                    "policy_conflict",
                    "分流策略存在冲突",
                    "规则身份或快照与预期不一致，已阻止自动覆盖。",
                ),
                (
                    before.cycle_failed,
                    next.cycle_failed,
                    "cycle_failed",
                    "线路检查失败",
                    "本轮检查或应用失败，详情请查看本地运行日志。",
                ),
                (
                    before.ipv6_degraded,
                    next.ipv6_degraded,
                    "ipv6_degraded",
                    "IPv6 状态异常",
                    "目标线路的 IPv6 地址、PD 或 LAN 前缀尚未满足双栈要求。",
                ),
            ] {
                if new && !old {
                    changes.push(event(kind, title, body.into()));
                }
            }
            if before.cycle_failed && !next.cycle_failed {
                changes.push(event(
                    "cycle_recovered",
                    "线路检查恢复",
                    "检查与策略协调已恢复。".into(),
                ));
            }
            if before.ipv6_degraded && !next.ipv6_degraded {
                changes.push(event(
                    "ipv6_recovered",
                    "IPv6 状态恢复",
                    "IPv6 配置与地址证据已恢复正常。".into(),
                ));
            }
            if applied_changes > 0 {
                changes.push(event(
                    "policy_synchronized",
                    "分流策略已同步",
                    format!("本轮已验证并应用 {applied_changes} 项策略变化。"),
                ));
            }
            next.initialized = true;
            stored.baseline = next;
            let now = now_ms();
            for mut value in changes {
                let affected_interface = value.context.affected_interface.clone();
                value.context =
                    if crate::notification::network_context::transition_kind(&value.kind) {
                        context
                            .clone()
                            .with_transition(&previous, &affected_interface)
                    } else {
                        context.clone()
                    };
                self.enqueue(stored, value, now);
            }
            stored.observed_network_context = Some(context);
        })
        .await?;
        self.wake.notify_one();
        Ok(())
    }

    pub async fn observe_ipv6(&self, values: Value) -> Result<(), String> {
        self.mutate(|stored| {
            let previous = stored
                .observed_network_context
                .clone()
                .unwrap_or_else(|| stored.network_context.clone());
            let mut next = stored.baseline.clone();
            ipv6_maps(&values, &mut next);
            let mut context = stored.network_context.clone();
            context.previous = None;
            context.affected_interface.clear();
            for line in &mut context.lines {
                let interface = line.interface.to_ascii_lowercase();
                line.ipv6 = next.ipv6.get(&interface).cloned().unwrap_or_default();
                line.prefix = next.prefixes.get(&interface).cloned().unwrap_or_default();
            }
            if let Some(line) = context
                .lines
                .iter()
                .find(|line| line.interface.eq_ignore_ascii_case(&context.interface))
            {
                context.ipv6 = line.ipv6.clone();
                context.prefix = line.prefix.clone();
            }
            stored.network_context = context.clone();
            let mut changes = Vec::new();
            if stored.baseline.initialized {
                compare_map(
                    &stored.baseline.ipv6,
                    &next.ipv6,
                    "wan_ipv6_changed",
                    "WAN IPv6 地址变化",
                    &mut changes,
                );
                compare_map(
                    &stored.baseline.prefixes,
                    &next.prefixes,
                    "ipv6_pd_changed",
                    "IPv6 PD 前缀变化",
                    &mut changes,
                );
            }
            stored.baseline.ipv6 = next.ipv6;
            stored.baseline.prefixes = next.prefixes;
            for mut value in changes {
                let affected_interface = value.context.affected_interface.clone();
                value.context = context
                    .clone()
                    .with_transition(&previous, &affected_interface);
                self.enqueue(stored, value, now_ms());
            }
            stored.observed_network_context = Some(context);
        })
        .await?;
        self.wake.notify_one();
        Ok(())
    }

    pub async fn dispatch_due(&self, now: u64) -> Result<bool, String> {
        if !self.config.enabled {
            return Ok(false);
        }
        if !self.stored.lock().await.queue.iter().any(|pending| {
            pending
                .targets
                .iter()
                .any(|target| !target.in_flight && target.next_ms <= now)
        }) {
            return Ok(false);
        }
        let selected = self
            .mutate(|stored| {
                for pending in &mut stored.queue {
                    if let Some(target) = pending
                        .targets
                        .iter_mut()
                        .find(|target| !target.in_flight && target.next_ms <= now)
                    {
                        target.in_flight = true;
                        return Some((pending.event.clone(), target.tag.clone(), target.attempts));
                    }
                }
                None
            })
            .await?;
        let Some((value, tag, previous_attempts)) = selected else {
            return Ok(false);
        };
        let enabled = self
            .config
            .channels
            .iter()
            .any(|channel| channel.enabled && channel.tag == tag);
        let result = if enabled {
            match tokio::time::timeout(
                Duration::from_millis(self.config.timeout_ms.saturating_mul(3)),
                self.sender.send_to(&value, std::slice::from_ref(&tag)),
            )
            .await
            {
                Ok(results) => results.into_iter().find(|result| result.tag == tag),
                Err(_) => None,
            }
        } else {
            None
        };
        let mut delivery = result.unwrap_or_else(|| DeliveryResult {
            tag: tag.clone(),
            provider: String::new(),
            success: false,
            error_code: Some("delivery_unavailable".into()),
        });
        delivery.error_code = delivery.error_code.map(|code| safe_text(&code, 64));
        if delivery.success {
            crate::logging::info(&format!("[通知投递] 渠道 {tag} 已投递事件 {}。", value.id));
        } else if enabled {
            crate::logging::warn(&format!(
                "[通知投递] 渠道 {tag} 失败：{}。",
                delivery
                    .error_code
                    .as_deref()
                    .unwrap_or("delivery_unavailable")
            ));
        }
        self.mutate(|stored| {
            let attempts = previous_attempts.saturating_add(1);
            if previous_attempts > 0 {
                stored.counters.retries = stored.counters.retries.saturating_add(1);
            }
            if delivery.success {
                stored.counters.delivered = stored.counters.delivered.saturating_add(1);
            } else if enabled {
                stored.counters.failed_attempts = stored.counters.failed_attempts.saturating_add(1);
            }
            if attempts >= MAX_ATTEMPTS && !delivery.success && enabled {
                stored.counters.exhausted = stored.counters.exhausted.saturating_add(1);
            }
            if let Some(pending) = stored
                .queue
                .iter_mut()
                .find(|pending| pending.event.id == value.id)
            {
                if delivery.success || attempts >= MAX_ATTEMPTS || !enabled {
                    pending.targets.retain(|target| target.tag != tag);
                } else if let Some(target) =
                    pending.targets.iter_mut().find(|target| target.tag == tag)
                {
                    target.attempts = attempts;
                    target.next_ms = now.saturating_add(
                        1000_u64.saturating_mul(1_u64 << attempts.saturating_sub(1)),
                    );
                    target.in_flight = false;
                }
                let remaining = pending
                    .targets
                    .iter()
                    .map(|target| target.tag.clone())
                    .collect();
                if let Some(history) = stored
                    .history
                    .iter_mut()
                    .find(|history| history.event.id == value.id)
                {
                    history.deliveries.retain(|old| old.tag != tag);
                    history.deliveries.push(delivery.clone());
                }
                finish_history(
                    stored,
                    &value.id,
                    remaining,
                    if delivery.success {
                        "delivered"
                    } else {
                        "failed"
                    },
                );
            }
            stored.queue.retain(|pending| !pending.targets.is_empty());
        })
        .await?;
        Ok(true)
    }

    pub async fn run(&self, cancel: CancellationToken) -> Result<(), String> {
        let permit = self
            .worker_gate
            .try_acquire()
            .map_err(|_| "Notification worker is already running".to_string())?;
        self.mutate(|stored| {
            for pending in &mut stored.queue {
                for target in &mut pending.targets {
                    target.in_flight = false;
                }
            }
        })
        .await?;
        loop {
            let worked = tokio::select! {
                () = cancel.cancelled() => break,
                result = self.dispatch_due(now_ms()) => result?,
            };
            if worked {
                continue;
            }
            tokio::select! {
                () = cancel.cancelled() => break,
                () = self.wake.notified() => {},
                () = tokio::time::sleep(Duration::from_millis(500)) => {},
            }
        }
        self.mutate(|stored| {
            for pending in &mut stored.queue {
                for target in &mut pending.targets {
                    target.in_flight = false;
                }
            }
        })
        .await?;
        drop(permit);
        Ok(())
    }

    pub async fn flush(&self, duration: Duration) -> Result<(), String> {
        if !self.config.enabled {
            return Ok(());
        }
        let flush = async {
            let permit = self
                .worker_gate
                .acquire()
                .await
                .map_err(|_| "Notification worker is unavailable".to_string())?;
            self.mutate(|stored| {
                for pending in &mut stored.queue {
                    for target in &mut pending.targets {
                        target.in_flight = false;
                    }
                }
            })
            .await?;
            while self.dispatch_due(now_ms()).await? {}
            drop(permit);
            Ok::<(), String>(())
        };
        let result = tokio::time::timeout(duration.min(Duration::from_secs(15)), flush).await;
        self.mutate(|stored| {
            for pending in &mut stored.queue {
                for target in &mut pending.targets {
                    target.in_flight = false;
                }
            }
        })
        .await?;
        match result {
            Ok(result) => result,
            Err(_) => Ok(()),
        }
    }

    pub async fn status(&self) -> Value {
        let stored = self.stored.lock().await;
        json!({ "enabled":self.config.enabled, "queued_events":stored.queue.len(), "queue_capacity":self.config.queue_capacity,
            "history_events":stored.history.len(), "counters":stored.counters,
            "channels":self.config.channels.iter().map(|channel|json!({"tag":channel.tag,"provider":channel.provider.name(),"enabled":channel.enabled})).collect::<Vec<_>>() })
    }

    pub async fn history(&self) -> Value {
        json!({ "events":self.stored.lock().await.history })
    }
}

fn finish_history(stored: &mut Stored, id: &str, remaining: Vec<String>, complete: &str) {
    if let Some(history) = stored
        .history
        .iter_mut()
        .find(|history| history.event.id == id)
    {
        history.pending_tags = remaining;
        history.status = if history.pending_tags.is_empty() {
            if complete == "delivered"
                && history.deliveries.iter().any(|delivery| !delivery.success)
            {
                "failed"
            } else {
                complete
            }
        } else {
            "queued"
        }
        .into();
    }
}

fn compare_map(
    old: &BTreeMap<String, String>,
    new: &BTreeMap<String, String>,
    kind: &str,
    title: &str,
    changes: &mut Vec<Event>,
) {
    for name in old
        .keys()
        .chain(new.keys())
        .collect::<std::collections::BTreeSet<_>>()
    {
        if old.get(name) != new.get(name) {
            let before = old
                .get(name)
                .filter(|value| crate::notification::network_context::available(value));
            let after = new
                .get(name)
                .filter(|value| crate::notification::network_context::available(value));
            let description = match (before, after) {
                (Some(before), Some(after)) => format!(
                    "{name}: {} → {}",
                    observed_label(Some(before)),
                    observed_label(Some(after))
                ),
                (None, Some(after)) => format!("{name}: {}", observed_label(Some(after))),
                (Some(_), None) => format!("{name} 的该项网络数据不再提供。"),
                (None, None) => format!("{name} 暂未返回新的网络数据。"),
            };
            let mut value = event(kind, title, description);
            value.context.affected_interface = name.to_ascii_uppercase();
            changes.push(value);
        }
    }
}

fn observed_label(value: Option<&String>) -> &str {
    match value.map(String::as_str) {
        Some("up") => "正常",
        Some("down") => "故障",
        Some("unknown") => "暂时无法判断",
        Some(value) => value,
        None => "",
    }
}

fn baseline(snapshot: &Value, previous: &Baseline) -> Baseline {
    let mut next = previous.clone();
    let primary = snapshot
        .get("primary_interface")
        .and_then(Value::as_str)
        .unwrap_or("wan1");
    let backup = snapshot
        .get("backup_interface")
        .and_then(Value::as_str)
        .unwrap_or("wan3");
    for (interface, field) in [(primary, "primary"), (backup, "backup")] {
        if let Some(value) = snapshot
            .pointer(&format!("/wan/{field}"))
            .and_then(Value::as_str)
            .filter(|value| ["up", "down", "unknown"].contains(value))
        {
            next.health.insert(interface.into(), value.into());
        }
    }
    if let Some(health) = snapshot.get("wan_health").and_then(Value::as_object) {
        for name in ["wan1", "wan2", "wan3"] {
            if let Some(value) = health
                .get(name)
                .and_then(Value::as_str)
                .filter(|value| ["up", "down", "unknown"].contains(value))
            {
                next.health.insert(name.into(), value.into());
            }
        }
    }
    if let Some(rows) = snapshot.get("interfaces").and_then(Value::as_array) {
        next.ipv4.clear();
        for row in rows {
            let name = row
                .get("interface")
                .or_else(|| row.get("name"))
                .and_then(Value::as_str)
                .unwrap_or_default();
            if !["wan1", "wan2", "wan3"].contains(&name) {
                continue;
            }
            if let Some(text) = row.get("ip_addr").and_then(Value::as_str) {
                if let Ok(IpAddr::V4(address)) =
                    text.split('/').next().unwrap_or_default().parse::<IpAddr>()
                {
                    if !address.is_unspecified() {
                        next.ipv4.insert(name.into(), address.to_string());
                    }
                }
            }
        }
    }
    if let Some(active) = snapshot
        .pointer("/wan/state/active")
        .and_then(Value::as_str)
        .filter(|value| ["primary", "backup"].contains(value))
    {
        next.active = active.into();
    }
    let target = snapshot
        .pointer("/wan/plan/target")
        .and_then(Value::as_str)
        .unwrap_or_default();
    next.pending = (!target.is_empty() && target != next.active)
        || (next.active == "primary"
            && next
                .health
                .get("wan1")
                .is_some_and(|health| health == "down")
            && snapshot
                .pointer("/wan/state/failed")
                .and_then(Value::as_u64)
                .is_some_and(|count| count > 0));
    let messages: Vec<&str> = snapshot
        .pointer("/wan/plan/blockers")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .chain(snapshot.pointer("/wan/last_error").and_then(Value::as_str))
        .chain(snapshot.pointer("/wan/ipv6_error").and_then(Value::as_str))
        .collect();
    next.conflict = messages.iter().any(|text| {
        let value = text.to_ascii_lowercase();
        ["conflict", "identity", "snapshot", "changed", "truncated"]
            .iter()
            .any(|word| value.contains(word))
    });
    let capability = snapshot
        .pointer("/wan/ipv6_capability")
        .and_then(Value::as_str)
        .unwrap_or_default();
    next.ipv6_degraded = ["required_missing", "degraded"].contains(&capability)
        || snapshot
            .pointer("/wan/ipv6_error")
            .and_then(Value::as_str)
            .is_some_and(|error| !error.is_empty())
        || messages
            .iter()
            .any(|text| text.to_ascii_lowercase().contains("ipv6"));
    if let Some(evidence) = snapshot
        .pointer("/wan/plan/ipv6_evidence")
        .and_then(Value::as_object)
    {
        next.ipv6_degraded |= [
            "wan_address_ready",
            "delegated_prefix_ready",
            "lan_prefix_ready",
        ]
        .iter()
        .any(|key| evidence.get(*key).and_then(Value::as_bool) == Some(false));
    }
    if let Some(rows) = snapshot.get("ipv6_addresses") {
        ipv6_maps(rows, &mut next);
    }
    next
}

fn ipv6_maps(values: &Value, baseline: &mut Baseline) {
    let Some(rows) = values
        .as_array()
        .or_else(|| values.get("data").and_then(Value::as_array))
    else {
        return;
    };
    baseline.ipv6.clear();
    baseline.prefixes.clear();
    for row in rows {
        let name = row
            .get("interface")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if !["wan1", "wan2", "wan3"].contains(&name) {
            continue;
        }
        if row.get("valid_lft").and_then(Value::as_u64) == Some(0)
            || row.get("valid_lft").and_then(Value::as_str) == Some("0")
        {
            continue;
        }
        for (field, map) in [
            ("dhcp6_ip_addr", &mut baseline.ipv6),
            ("dhcp6_prefix1", &mut baseline.prefixes),
        ] {
            if let Some(value) = row.get(field).and_then(Value::as_str) {
                if let Ok(network) = value.parse::<ipnet::Ipv6Net>() {
                    map.insert(name.into(), network.to_string());
                }
            }
        }
    }
}
