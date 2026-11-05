use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

use async_trait::async_trait;
use ikb_failover::{
    events::{EventSender, Events},
    notification::{ChannelConfig, DeliveryResult, Event, NotificationConfig, ProviderConfig},
};
use serde_json::{Value, json};

#[derive(Default)]
struct Sender {
    calls: Mutex<BTreeMap<String, u64>>,
    failures: u64,
}

#[async_trait]
impl EventSender for Sender {
    async fn send_to(&self, _event: &Event, tags: &[String]) -> Vec<DeliveryResult> {
        let mut calls = self.calls.lock().unwrap();
        tags.iter()
            .map(|tag| {
                let count = calls.entry(tag.clone()).or_default();
                *count += 1;
                let success = tag != "flaky" || *count > self.failures;
                DeliveryResult {
                    tag: tag.clone(),
                    provider: "mock".into(),
                    success,
                    error_code: if success {
                        None
                    } else {
                        Some("mock_failure".into())
                    },
                }
            })
            .collect()
    }
}

fn config() -> NotificationConfig {
    NotificationConfig {
        enabled: true,
        cooldown_ms: 60_000,
        channels: ["fast", "flaky"]
            .iter()
            .map(|tag| ChannelConfig {
                tag: (*tag).into(),
                enabled: true,
                name: String::new(),
                message_format: None,
                mp_config: Default::default(),
                switchs: Vec::new(),
                provider: ProviderConfig::Telegram {
                    api_url: "https://api.telegram.org".into(),
                    bot_token_env: "UNUSED_MOCK_TOKEN".into(),
                    chat_id: "1".into(),
                    message_thread_id: None,
                },
            })
            .collect(),
        ..NotificationConfig::default()
    }
}

fn snapshot(ip: &str, primary: &str, active: &str, degraded: bool) -> Value {
    json!({"wan":{"primary":primary,"backup":"up","state":{"active":active,"failed":0},"last_error":"",
        "ipv6_capability":if degraded {"required_missing"} else {"configuration_verified"},
        "plan":{"target":active,"blockers":[],"ipv6_evidence":{"wan_address_ready":!degraded,"delegated_prefix_ready":!degraded,"lan_prefix_ready":!degraded}}},
        "interfaces":[{"name":"wan1","ip_addr":ip},{"name":"wan2","ip_addr":"192.0.2.2"},{"name":"wan3","ip_addr":"100.64.1.2"}],
        "wan_health":{"wan1":primary,"wan2":"up","wan3":"up"},
        "ipv6_addresses":[{"interface":"wan1","dhcp6_ip_addr":"2001:db8:100::1/64","dhcp6_prefix1":"2001:db8:100::/64","valid_lft":3600}]})
}

#[tokio::test]
async fn baseline_survives_restart_and_only_reports_changes() {
    let directory = tempfile::tempdir().unwrap();
    let sender = Arc::new(Sender::default());
    let config = config();
    let events = Events::open_with_sender(&config, directory.path(), sender.clone())
        .await
        .unwrap();
    events
        .observe(&snapshot("100.64.1.1", "up", "primary", false), false, 0)
        .await
        .unwrap();
    assert_eq!(
        events.history().await["events"].as_array().unwrap().len(),
        0
    );
    drop(events);
    let events = Events::open_with_sender(&config, directory.path(), sender)
        .await
        .unwrap();
    events
        .observe(&snapshot("100.64.1.1", "up", "primary", false), false, 0)
        .await
        .unwrap();
    assert_eq!(
        events.history().await["events"].as_array().unwrap().len(),
        0
    );
    events
        .observe(&snapshot("100.64.1.3", "down", "backup", true), true, 2)
        .await
        .unwrap();
    let history = events.history().await;
    let kinds: Vec<&str> = history["events"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["event"]["kind"].as_str().unwrap())
        .collect();
    for kind in [
        "wan_ipv4_changed",
        "wan_health_changed",
        "egress_switched",
        "cycle_failed",
        "ipv6_degraded",
        "policy_synchronized",
    ] {
        assert!(kinds.contains(&kind), "missing {kind}");
    }
    events
        .observe(&snapshot("100.64.1.3", "up", "primary", false), false, 0)
        .await
        .unwrap();
    let history = events.history().await;
    let kinds: Vec<&str> = history["events"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["event"]["kind"].as_str().unwrap())
        .collect();
    for kind in ["egress_recovered", "cycle_recovered", "ipv6_recovered"] {
        assert!(kinds.contains(&kind));
    }
}

#[tokio::test]
async fn cooldown_filters_and_queue_capacity_are_bounded() {
    let directory = tempfile::tempdir().unwrap();
    let mut config = config();
    config.queue_capacity = 1;
    config.events = vec!["service_started".into()];
    let events = Events::open_with_sender(&config, directory.path(), Arc::new(Sender::default()))
        .await
        .unwrap();
    events
        .emit("service_started", "启动", "ready")
        .await
        .unwrap();
    events
        .emit("service_started", "启动", "ready")
        .await
        .unwrap();
    events
        .emit("service_started", "启动", "ready again")
        .await
        .unwrap();
    events
        .emit("cycle_failed", "失败", "generic status")
        .await
        .unwrap();
    let status = events.status().await;
    assert_eq!(status["queued_events"], 1);
    assert_eq!(status["counters"]["suppressed"], 1);
    assert_eq!(status["counters"]["dropped"], 1);
    assert_eq!(status["counters"]["filtered"], 1);
}

#[tokio::test]
async fn successful_channels_are_not_retried_and_failed_channel_resumes_after_restart() {
    let directory = tempfile::tempdir().unwrap();
    let sender = Arc::new(Sender {
        failures: 1,
        ..Sender::default()
    });
    let mut config = config();
    // Service events require an explicit selection; test delivery retries here.
    config.events = vec!["service_started".into()];
    let events = Events::open_with_sender(&config, directory.path(), sender.clone())
        .await
        .unwrap();
    events
        .emit("service_started", "启动", "ready")
        .await
        .unwrap();
    assert!(events.dispatch_due(u64::MAX).await.unwrap());
    assert!(events.dispatch_due(u64::MAX).await.unwrap());
    assert_eq!(events.status().await["queued_events"], 1);
    drop(events);
    let events = Events::open_with_sender(&config, directory.path(), sender.clone())
        .await
        .unwrap();
    assert!(events.dispatch_due(u64::MAX).await.unwrap());
    assert_eq!(events.status().await["queued_events"], 0);
    let calls = sender.calls.lock().unwrap();
    assert_eq!(calls.get("fast"), Some(&1));
    assert_eq!(calls.get("flaky"), Some(&2));
}

#[tokio::test]
async fn retries_stop_after_five_attempts_and_disabled_mode_never_sends() {
    let directory = tempfile::tempdir().unwrap();
    let sender = Arc::new(Sender {
        failures: u64::MAX,
        ..Sender::default()
    });
    let mut config = config();
    let events = Events::open_with_sender(&config, directory.path(), sender.clone())
        .await
        .unwrap();
    events
        .emit("cycle_failed", "检查失败", "generic status")
        .await
        .unwrap();
    while events.dispatch_due(u64::MAX).await.unwrap() {}
    assert_eq!(events.status().await["counters"]["exhausted"], 1);
    assert_eq!(sender.calls.lock().unwrap().get("flaky"), Some(&5));
    drop(events);
    config.enabled = false;
    let events = Events::open_with_sender(&config, directory.path(), sender.clone())
        .await
        .unwrap();
    events
        .emit("service_started", "启动", "generic status")
        .await
        .unwrap();
    assert!(!events.dispatch_due(u64::MAX).await.unwrap());
    assert_eq!(sender.calls.lock().unwrap().get("fast"), Some(&1));
}

#[tokio::test]
async fn state_contains_normalized_data_and_redacts_urls() {
    let directory = tempfile::tempdir().unwrap();
    let config = config();
    let events = Events::open_with_sender(&config, directory.path(), Arc::new(Sender::default()))
        .await
        .unwrap();
    let mut value = snapshot("100.64.1.1", "up", "primary", false);
    value["wan"]["last_error"] =
        json!("raw private error https://example.test/secret?token=credentials");
    events.observe(&value, true, 0).await.unwrap();
    events
        .emit(
            "service_failed",
            "运行失败",
            "See https://example.test/private?token=credentials",
        )
        .await
        .unwrap();
    let saved = tokio::fs::read_to_string(directory.path().join("notifications.json"))
        .await
        .unwrap();
    assert!(!saved.contains("raw private error"));
    assert!(!saved.contains("example.test"));
    assert!(!saved.contains("credentials"));
    assert!(saved.contains("100.64.1.1"));
}

#[tokio::test]
async fn flush_delivers_shutdown_and_detects_runtime_ipv6_errors() {
    let directory = tempfile::tempdir().unwrap();
    let sender = Arc::new(Sender::default());
    let config = config();
    let events = Events::open_with_sender(&config, directory.path(), sender.clone())
        .await
        .unwrap();
    events
        .observe(&snapshot("100.64.1.1", "up", "backup", false), false, 0)
        .await
        .unwrap();
    let mut degraded = snapshot("100.64.1.1", "down", "backup", false);
    degraded["wan"]["state"]["failed"] = json!(10);
    degraded["wan"]["ipv6_capability"] = json!("degraded");
    degraded["wan"]["ipv6_error"] = json!("PD settlement timeout");
    events.observe(&degraded, false, 0).await.unwrap();
    events
        .emit("service_stopped", "服务退出", "正常退出")
        .await
        .unwrap();
    events
        .flush(std::time::Duration::from_secs(1))
        .await
        .unwrap();
    assert_eq!(events.status().await["queued_events"], 0);
    let history = events.history().await;
    let kinds: Vec<&str> = history["events"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["event"]["kind"].as_str().unwrap())
        .collect();
    assert!(kinds.contains(&"ipv6_degraded"));
    assert!(!kinds.contains(&"failover_pending"));
    assert!(kinds.contains(&"service_stopped"));
}

#[tokio::test]
async fn first_cycle_switch_is_reported_without_initial_address_noise() {
    let directory = tempfile::tempdir().unwrap();
    let config = config();
    let events = Events::open_with_sender(&config, directory.path(), Arc::new(Sender::default()))
        .await
        .unwrap();
    events.seed_active("primary").await.unwrap();
    events
        .observe(&snapshot("100.64.1.1", "down", "backup", false), false, 0)
        .await
        .unwrap();
    let history = events.history().await;
    let kinds: Vec<&str> = history["events"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["event"]["kind"].as_str().unwrap())
        .collect();
    assert!(kinds.contains(&"egress_switched"));
    assert!(!kinds.contains(&"wan_ipv4_changed"));
    assert!(!kinds.contains(&"wan_health_changed"));
    drop(events);
    let events = Events::open_with_sender(&config, directory.path(), Arc::new(Sender::default()))
        .await
        .unwrap();
    events.seed_active("primary").await.unwrap();
    events
        .observe(&snapshot("100.64.1.1", "down", "backup", false), false, 0)
        .await
        .unwrap();
    assert_eq!(
        events.history().await["events"].as_array().unwrap().len(),
        kinds.len()
    );
}

#[tokio::test]
async fn unchanged_observations_and_idle_dispatch_do_not_rewrite_state() {
    let directory = tempfile::tempdir().unwrap();
    let config = config();
    let events = Events::open_with_sender(&config, directory.path(), Arc::new(Sender::default()))
        .await
        .unwrap();
    let value = snapshot("100.64.1.1", "up", "primary", false);
    events.observe(&value, false, 0).await.unwrap();
    let path = directory.path().join("notifications.json");
    let modified = tokio::fs::metadata(&path)
        .await
        .unwrap()
        .modified()
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    events.observe(&value, false, 0).await.unwrap();
    assert!(!events.dispatch_due(u64::MAX).await.unwrap());
    assert_eq!(
        tokio::fs::metadata(path).await.unwrap().modified().unwrap(),
        modified
    );
}

#[tokio::test]
async fn corrupt_notification_state_is_preserved_and_service_can_continue() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("notifications.json");
    tokio::fs::write(&path, b"invalid notification state")
        .await
        .unwrap();
    let config = config();
    let events = Events::open_with_sender(&config, directory.path(), Arc::new(Sender::default()))
        .await
        .unwrap();
    let backups: Vec<_> = std::fs::read_dir(directory.path())
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("notifications.corrupt-")
        })
        .collect();
    assert_eq!(backups.len(), 1);
    assert_eq!(
        tokio::fs::read(&backups[0]).await.unwrap(),
        b"invalid notification state"
    );
    events
        .emit("service_started", "启动", "ready")
        .await
        .unwrap();
    events
        .flush(std::time::Duration::from_secs(1))
        .await
        .unwrap();
    assert_eq!(events.status().await["queued_events"], 0);
    assert!(serde_json::from_slice::<Value>(&tokio::fs::read(path).await.unwrap()).is_ok());
}
