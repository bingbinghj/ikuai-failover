use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};

use async_trait::async_trait;
use ikb_failover::{
    api::RouterApi,
    config::{Config, RulePair, WanConfig},
    journal::{Journal, Store},
    policy::{self, Resource},
    runtime::Runtime,
    state::{Egress, Health, WanState},
};
use serde_json::{Value, json};
use tokio::sync::RwLock;

struct MockRouter {
    data: RwLock<HashMap<Resource, Vec<Value>>>,
    primary_up: AtomicBool,
    backup_known: AtomicBool,
    writes: AtomicUsize,
    fail_at: AtomicUsize,
    external_change_on_failure: AtomicBool,
}

impl MockRouter {
    fn new() -> Self {
        Self {
            data: RwLock::new(HashMap::from([(
                Resource::LoadBalanceRules,
                vec![
                    json!({"id":1,"tagname":"wan1","interface":"wan1","enabled":"yes","isp_name":"IKB-domestic","weight":"1","mode":0,"comment":"owner label"}),
                    json!({"id":4,"tagname":"wan3","interface":"wan3","enabled":"no","isp_name":"IKB-domestic","weight":"1","mode":0,"comment":"owner label"}),
                    json!({"id":2,"tagname":"IKB-google","interface":"wan2","enabled":"yes","isp_name":"IKB-google","weight":"1","mode":0,"comment":"IkuaiBypass"}),
                ],
            )])),
            primary_up: AtomicBool::new(true),
            backup_known: AtomicBool::new(true),
            writes: AtomicUsize::new(0),
            fail_at: AtomicUsize::new(0),
            external_change_on_failure: AtomicBool::new(false),
        }
    }
    async fn snapshot(&self) -> Vec<Value> {
        self.data.read().await[&Resource::LoadBalanceRules].clone()
    }
}

#[async_trait]
impl RouterApi for MockRouter {
    async fn get(&self, path: &str) -> Result<Value, String> {
        if path == "/interfaces/wan-config" {
            return Ok(json!({"data":[
                {"name":"wan1","default_route":0},
                {"name":"wan2","default_route":1},
                {"name":"wan3","default_route":0}
            ]}));
        }
        if path == "/monitoring/interfaces-status" {
            return Ok(
                json!({"iface_check":[{"interface":"wan1","result":if self.primary_up.load(Ordering::SeqCst) {"success"} else {"failed"}},
                {"interface":"wan3","result":if self.backup_known.load(Ordering::SeqCst) {"success"} else {"unknown"}}]}),
            );
        }
        let data = self.data.read().await;
        let resource = if path.starts_with(Resource::LoadBalanceRules.path()) {
            Resource::LoadBalanceRules
        } else if path.starts_with(Resource::DomainRules.path()) {
            if data.contains_key(&Resource::DomainRuleState) {
                Resource::DomainRuleState
            } else {
                Resource::DomainRules
            }
        } else {
            Resource::FiveTupleRules
        };
        let rows = data.get(&resource).ok_or("Unknown mock resource")?;
        Ok(json!({"data":rows,"total":rows.len()}))
    }
    async fn put(&self, resource: Resource, body: &Value) -> Result<(), String> {
        let call = self.writes.fetch_add(1, Ordering::SeqCst).saturating_add(1);
        let mut data = self.data.write().await;
        if call == self.fail_at.load(Ordering::SeqCst) {
            if self.external_change_on_failure.load(Ordering::SeqCst) {
                for row in data.get_mut(&resource).ok_or("Unknown mock resource")? {
                    if row["id"] == 4 {
                        row["comment"] = json!("external writer");
                    }
                }
            }
            return Err("Injected update failure".into());
        }
        let row = data
            .get_mut(&resource)
            .ok_or("Unknown mock resource")?
            .iter_mut()
            .find(|row| row["id"] == body["id"])
            .ok_or("Mock rule absent")?;
        *row = body.clone();
        Ok(())
    }
}

fn wan() -> WanConfig {
    WanConfig {
        enabled: true,
        primary: "wan1".into(),
        backup: "wan3".into(),
        preserved_default: "wan2".into(),
        check_interval_ms: 100,
        failure_threshold: 3,
        recovery_threshold: 5,
        stable_hold_ms: 1000,
        ipv6_enabled: false,
        ipv6: None,
        domain_rules: Vec::new(),
        rule_pairs: vec![RulePair {
            resource: Resource::LoadBalanceRules,
            primary_id: 1,
            backup_id: 4,
            primary_tag: "wan1".into(),
            backup_tag: "wan3".into(),
        }],
    }
}

#[test]
fn native_health_and_hysteresis_require_stable_recovery_before_failback() {
    let config = wan();
    let mut state = WanState::default();
    for _ in 0..2 {
        assert_eq!(
            state.observe(Health::Down, Health::Up, &config, 2000),
            Egress::Primary
        );
    }
    assert_eq!(
        state.observe(Health::Down, Health::Up, &config, 2000),
        Egress::Backup
    );
    state.commit(Egress::Backup, 2000);
    for _ in 0..5 {
        assert_eq!(
            state.observe(Health::Up, Health::Up, &config, 2500),
            Egress::Backup
        );
    }
    assert_eq!(
        state.observe(Health::Up, Health::Up, &config, 3100),
        Egress::Primary
    );
    state.commit(Egress::Primary, 3100);
    assert_eq!(
        state.observe(Health::Unknown, Health::Up, &config, 10000),
        Egress::Primary
    );
    assert_eq!(
        ikb_failover::state::health(&json!({"iface_check":[]}), "wan1"),
        Health::Unknown
    );
}

#[tokio::test]
async fn plan_changes_only_enabled_fields_of_explicit_physical_rule_ids() {
    let api = MockRouter::new();
    let original = api.snapshot().await;
    let plan = policy::build(&api, &wan(), Egress::Backup).await.unwrap();
    assert_eq!(plan.changes.len(), 2);
    assert_eq!(plan.changes[0].id, 4);
    assert!(plan.blockers.is_empty());
    for change in &plan.changes {
        let mut expected = change.before.clone();
        expected["enabled"] = change.after["enabled"].clone();
        assert_eq!(expected, change.after);
        assert_eq!(change.after["isp_name"], "IKB-domestic");
    }
    assert_eq!(api.snapshot().await, original);
    let mut config = wan();
    config.ipv6_enabled = true;
    assert!(
        !policy::build(&api, &config, Egress::Backup)
            .await
            .unwrap()
            .ipv6_error
            .is_empty()
    );
}

#[tokio::test]
async fn partial_update_failure_rolls_back_and_preserves_wan2_rule() {
    let api = MockRouter::new();
    let before = api.snapshot().await;
    api.fail_at.store(2, Ordering::SeqCst);
    let config = wan();
    let plan = policy::build(&api, &config, Egress::Backup).await.unwrap();
    let directory = tempfile::tempdir().unwrap();
    let store = Store::open(directory.path()).await.unwrap();
    assert!(store.apply(&api, &plan, &config, 10000).await.is_err());
    assert_eq!(api.snapshot().await, before);
    assert!(!store.has_pending().await.unwrap());
    assert_eq!(api.writes.load(Ordering::SeqCst), 3);
}

#[tokio::test]
async fn external_edit_blocks_rollback_without_overwriting_other_writer() {
    let api = MockRouter::new();
    api.fail_at.store(2, Ordering::SeqCst);
    api.external_change_on_failure.store(true, Ordering::SeqCst);
    let config = wan();
    let plan = policy::build(&api, &config, Egress::Backup).await.unwrap();
    let directory = tempfile::tempdir().unwrap();
    let store = Store::open(directory.path()).await.unwrap();
    assert!(store.apply(&api, &plan, &config, 10000).await.is_err());
    assert!(store.has_pending().await.unwrap());
    assert!(store.recover(&api, &config).await.is_err());
    assert_eq!(api.snapshot().await[1]["comment"], "external writer");
    assert_eq!(api.writes.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn crash_journal_recovery_restores_partial_changes_and_checks_allowlist() {
    let api = MockRouter::new();
    let before = api.snapshot().await;
    let config = wan();
    let plan = policy::build(&api, &config, Egress::Backup).await.unwrap();
    let directory = tempfile::tempdir().unwrap();
    let store = Store::open(directory.path()).await.unwrap();
    let journal = Journal {
        phase: "prepared".into(),
        target: Egress::Backup,
        changed_ms: 10000,
        changes: plan.changes.clone(),
        error: String::new(),
    };
    tokio::fs::write(
        directory.path().join("journal.json"),
        serde_json::to_vec(&journal).unwrap(),
    )
    .await
    .unwrap();
    api.put(plan.changes[0].resource, &plan.changes[0].after)
        .await
        .unwrap();
    store.recover(&api, &config).await.unwrap();
    assert_eq!(api.snapshot().await, before);
    let mut forged = journal;
    forged.changes[0].after["interface"] = json!("wan2");
    tokio::fs::write(
        directory.path().join("journal.json"),
        serde_json::to_vec(&forged).unwrap(),
    )
    .await
    .unwrap();
    let writes = api.writes.load(Ordering::SeqCst);
    assert!(store.recover(&api, &config).await.is_err());
    assert_eq!(api.writes.load(Ordering::SeqCst), writes);
}

#[tokio::test]
async fn committed_journal_recovers_active_state_and_lock_excludes_second_daemon() {
    let api = MockRouter::new();
    let config = wan();
    let plan = policy::build(&api, &config, Egress::Backup).await.unwrap();
    let directory = tempfile::tempdir().unwrap();
    let store = Store::open(directory.path()).await.unwrap();
    assert!(Store::open(directory.path()).await.is_err());
    store.apply(&api, &plan, &config, 10000).await.unwrap();
    assert_eq!(store.state().await.unwrap().active, Egress::Backup);
    assert_eq!(api.snapshot().await[2]["interface"], "wan2");
}

#[tokio::test]
async fn dry_run_produces_failover_plan_with_zero_router_writes() {
    let api = Arc::new(MockRouter::new());
    api.primary_up.store(false, Ordering::SeqCst);
    let directory = tempfile::tempdir().unwrap();
    let mut config: Config = serde_yaml::from_str(include_str!("fixtures/config.yml")).unwrap();
    config.management.apply = false;
    config.management.state_dir = directory.path().to_path_buf();
    config.wan = wan();
    config.wan.failure_threshold = 1;
    config.wan.stable_hold_ms = 0;
    let runtime = Runtime::new(config, api.clone()).await.unwrap();
    runtime.cycle().await.unwrap();
    let status = runtime.snapshot().await;
    assert_eq!(status["wan"]["plan"]["target"], "backup");
    assert_eq!(
        status["wan"]["plan"]["changes"].as_array().unwrap().len(),
        2
    );
    assert_eq!(api.writes.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn primary_recovery_fails_back_even_when_backup_health_is_unknown() {
    let api = Arc::new(MockRouter::new());
    api.backup_known.store(false, Ordering::SeqCst);
    {
        let mut rows = api.data.write().await;
        rows.get_mut(&Resource::LoadBalanceRules).unwrap()[0]["enabled"] = json!("no");
        rows.get_mut(&Resource::LoadBalanceRules).unwrap()[1]["enabled"] = json!("yes");
    }
    let directory = tempfile::tempdir().unwrap();
    let state = WanState {
        active: Egress::Backup,
        ..WanState::default()
    };
    tokio::fs::write(
        directory.path().join("state.json"),
        serde_json::to_vec(&state).unwrap(),
    )
    .await
    .unwrap();
    let mut config: Config = serde_yaml::from_str(include_str!("fixtures/config.yml")).unwrap();
    config.management.apply = true;
    config.management.state_dir = directory.path().to_path_buf();
    config.wan = wan();
    config.wan.recovery_threshold = 1;
    config.wan.stable_hold_ms = 0;
    let runtime = Runtime::new(config, api.clone()).await.unwrap();
    runtime.cycle().await.unwrap();
    assert_eq!(
        runtime.snapshot().await["wan"]["state"]["active"],
        "primary"
    );
    assert_eq!(api.snapshot().await[0]["enabled"], "yes");
    assert_eq!(api.snapshot().await[1]["enabled"], "no");
}

#[tokio::test]
async fn mismatched_primary_and_backup_traffic_selectors_block_apply() {
    let api = MockRouter::new();
    api.data
        .write()
        .await
        .get_mut(&Resource::LoadBalanceRules)
        .unwrap()[1]["isp_name"] = json!("different-traffic");
    let error = policy::build(&api, &wan(), Egress::Backup)
        .await
        .err()
        .unwrap();
    assert!(error.contains("same traffic"));
    assert_eq!(api.writes.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn domain_route_only_changes_interface_and_disabled_or_proxy_edits_block_apply() {
    let api = MockRouter::new();
    let domain = json!({"id":4,"tagname":"MyDirect","interface":"wan1,wan3","enabled":"yes",
        "src_addr":{"custom":["192.0.2.1-192.0.2.252"],"object":[]},"domain":{"custom":["example.com"],"object":[]},
        "prio":31,"comment":"IkuaiBypass","time":{"custom":[],"object":[]}});
    api.data
        .write()
        .await
        .insert(Resource::DomainRules, vec![domain.clone()]);
    let mut settings = wan();
    settings.rule_pairs.clear();
    settings
        .domain_rules
        .push(ikb_failover::config::DomainBinding {
            id: 4,
            tag: "MyDirect".into(),
            source: "192.0.2.1-192.0.2.252".into(),
        });
    let plan = policy::build(&api, &settings, Egress::Backup)
        .await
        .unwrap();
    assert_eq!(plan.changes.len(), 1);
    let full = policy::full_domain_payload(&domain, &plan.changes[0].after).unwrap();
    let mut expected = domain.clone();
    expected["interface"] = json!("wan3");
    assert_eq!(full, expected);
    let mut disabled = domain.clone();
    disabled["enabled"] = json!("no");
    api.data
        .write()
        .await
        .insert(Resource::DomainRules, vec![disabled.clone()]);
    assert!(
        policy::build(&api, &settings, Egress::Backup)
            .await
            .is_err()
    );
    assert!(policy::full_domain_payload(&disabled, &plan.changes[0].after).is_err());
    let mut moved = domain;
    moved["interface"] = json!("wan2");
    api.data
        .write()
        .await
        .insert(Resource::DomainRules, vec![moved]);
    // The planner owns the configured WAN pair; the payload helper only validates WAN syntax.
    assert!(
        policy::build(&api, &settings, Egress::Backup)
            .await
            .is_err()
    );
    assert_eq!(api.writes.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn completed_journal_does_not_prevent_new_explicit_configuration() {
    let api = MockRouter::new();
    let settings = wan();
    let plan = policy::build(&api, &settings, Egress::Backup)
        .await
        .unwrap();
    let directory = tempfile::tempdir().unwrap();
    let store = Store::open(directory.path()).await.unwrap();
    store.apply(&api, &plan, &settings, 10000).await.unwrap();
    let mut updated = settings;
    updated.rule_pairs.clear();
    let writes = api.writes.load(Ordering::SeqCst);
    store.recover(&api, &updated).await.unwrap();
    assert_eq!(api.writes.load(Ordering::SeqCst), writes);
}

#[tokio::test]
async fn unavailable_ipv6_credentials_do_not_block_ipv4_failover_or_persisted_state() {
    let api = Arc::new(MockRouter::new());
    api.primary_up.store(false, Ordering::SeqCst);
    let directory = tempfile::tempdir().unwrap();
    let mut settings: Config = serde_yaml::from_str(include_str!("fixtures/config.yml")).unwrap();
    settings.management.apply = true;
    settings.management.state_dir = directory.path().to_path_buf();
    settings.wan = wan();
    settings.wan.ipv6_enabled = true;
    settings.wan.failure_threshold = 1;
    settings.wan.stable_hold_ms = 0;
    let runtime = Runtime::new(settings.clone(), api.clone()).await.unwrap();
    runtime.cycle().await.unwrap();
    let status = runtime.snapshot().await;
    assert_eq!(status["wan"]["state"]["active"], "backup");
    assert_eq!(status["wan"]["ipv6_capability"], "degraded");
    assert!(status["wan"]["last_error"].as_str().unwrap().is_empty());
    assert_eq!(api.snapshot().await[0]["enabled"], "no");
    assert_eq!(api.snapshot().await[1]["enabled"], "yes");
    drop(runtime);
    let restarted = Runtime::new(settings, api).await.unwrap();
    assert_eq!(
        restarted.snapshot().await["wan"]["state"]["active"],
        "backup"
    );
}

#[tokio::test]
async fn paired_domain_rules_only_toggle_enabled_and_preserve_selectors_and_interfaces() {
    let api = MockRouter::new();
    let primary = json!({"id":4,"tagname":"primary-direct","interface":"wan1","enabled":"yes",
        "src_addr":{"custom":["192.0.2.1-192.0.2.252"],"object":[]},"domain":{"custom":["example.com"],"object":[]},
        "prio":31,"comment":"IkuaiBypass","time":{"custom":[],"object":[]}});
    let mut backup = primary.clone();
    backup["id"] = json!(5);
    backup["tagname"] = json!("backup-direct");
    backup["interface"] = json!("wan3");
    api.data.write().await.insert(
        Resource::DomainRuleState,
        vec![primary.clone(), backup.clone()],
    );
    let mut settings = wan();
    settings.rule_pairs = vec![RulePair {
        resource: Resource::DomainRuleState,
        primary_id: 4,
        backup_id: 5,
        primary_tag: "primary-direct".into(),
        backup_tag: "backup-direct".into(),
    }];
    let plan = policy::build(&api, &settings, Egress::Primary)
        .await
        .unwrap();
    assert_eq!(plan.changes.len(), 1);
    assert_eq!(plan.changes[0].resource, Resource::DomainRuleState);
    assert_eq!(plan.changes[0].id, 5);
    let mut expected = backup;
    expected["enabled"] = json!("no");
    assert_eq!(plan.changes[0].after, expected);
    let directory = tempfile::tempdir().unwrap();
    let store = Store::open(directory.path()).await.unwrap();
    store.apply(&api, &plan, &settings, 1000).await.unwrap();
    let failover = policy::build(&api, &settings, Egress::Backup)
        .await
        .unwrap();
    assert_eq!(failover.changes.len(), 2);
    store.apply(&api, &failover, &settings, 2000).await.unwrap();
    let rows = api.data.read().await;
    assert_eq!(rows[&Resource::DomainRuleState][0]["enabled"], "no");
    assert_eq!(rows[&Resource::DomainRuleState][1]["enabled"], "yes");
    assert_eq!(rows[&Resource::DomainRuleState][0]["interface"], "wan1");
    assert_eq!(rows[&Resource::DomainRuleState][1]["interface"], "wan3");
    assert_eq!(
        rows[&Resource::DomainRuleState][0]["domain"],
        primary["domain"]
    );
    drop(rows);
    api.data
        .write()
        .await
        .get_mut(&Resource::DomainRuleState)
        .unwrap()[1]["domain"]["custom"] = json!(["different.example"]);
    assert!(
        policy::build(&api, &settings, Egress::Primary)
            .await
            .is_err()
    );
}
