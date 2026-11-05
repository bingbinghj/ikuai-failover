use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::Duration,
};

use async_trait::async_trait;
use ikb_failover::{
    api::RouterApi,
    config::Config,
    journal::{Journal, Store},
    policy::{self, Resource},
    runtime::Runtime,
    state::Egress,
};
use serde_json::{Value, json};
use tokio::sync::RwLock;

#[derive(Clone, Copy)]
enum Mode {
    Ready,
    NoPrefix,
    StalePrefix,
    BadLan,
}

struct Model {
    rows: RwLock<Vec<Value>>,
    wan: RwLock<Value>,
    lan: RwLock<Value>,
    primary_up: AtomicBool,
    calls: RwLock<Vec<Resource>>,
    leases: AtomicU64,
    mode: Mode,
}

fn wan(interface: &str, mode: Mode) -> Value {
    // Mock 6to4-style addresses encode documentation IPv4 hosts, not a deployment.
    let backup = interface == "wan3";
    let prefix = if backup && matches!(mode, Mode::NoPrefix) {
        ""
    } else if backup && !matches!(mode, Mode::StalePrefix) {
        "2002:c000:0202:3::/64"
    } else {
        "2002:c000:0201:1::/64"
    };
    json!({"id":1,"tagname":"wan6","enabled":"yes","interface":interface,"internet":"dhcp",
        "prefix":"auto","prefix_hint":"","force_prefix":0,"force_gen_duid":1,"ipv6_addr":"","ipv6_gateway":"",
        "dhcp6_ip_addr":if backup {"2002:c000:0202:300::1/64"} else {"2002:c000:0201:100::1/64"},"dhcp6_prefix1":prefix,
        "dhcp6_ip_gateway":"fe80::1","dhcp6_dns1":"2001:db8::53","dhcp6_dns2":"","link_addr":"fe80::2",
        "preferred_lft":"1000","valid_lft":"1000"})
}

fn lan(parent: &str, mode: Mode) -> Value {
    let address = if parent == "wan3" && !matches!(mode, Mode::BadLan) {
        "2002:c000:0202:3::1001/64"
    } else {
        "2002:c000:0201:1::1001/64"
    };
    json!({"id":1,"tagname":"lan6","enabled":"yes","interface":"lan1","internet":"dhcp","parent":parent,
        "prefix_len":"auto","ipv6_addr":address,"dhcpv6":1,"ra_flags":"1","ra_static":0,"use_dns6":0,
        "ipv6_dns1":"","ipv6_dns2":"","leasetime":"120","ra_mtu_set":0,"ra_mtu":1480,"linkaddr":"fe80::3/64"})
}

impl Model {
    fn new(mode: Mode) -> Self {
        Self {
            rows: RwLock::new(vec![
                json!({"id":1,"tagname":"wan1","enabled":"yes","interface":"wan1","isp_name":"IKB-domestic","mode":0,"weight":"1","comment":"primary"}),
                json!({"id":4,"tagname":"wan3","enabled":"no","interface":"wan3","isp_name":"IKB-domestic","mode":0,"weight":"1","comment":"backup"}),
                json!({"id":2,"tagname":"IKB-google","enabled":"yes","interface":"wan2","isp_name":"IKB-google","mode":0,"weight":"1","comment":"IkuaiBypass"}),
            ]),
            wan: RwLock::new(wan("wan1", mode)),
            lan: RwLock::new(lan("wan1", mode)),
            primary_up: AtomicBool::new(false),
            calls: RwLock::new(Vec::new()),
            leases: AtomicU64::new(0),
            mode,
        }
    }
}

#[async_trait]
impl RouterApi for Model {
    fn ipv6_supported(&self) -> bool {
        true
    }
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
                json!({"iface_check":[{"interface":"wan1","result":if self.primary_up.load(Ordering::SeqCst){"success"}else{"failed"}},
            {"interface":"wan3","result":"success"}]}),
            );
        }
        if path == "/internal/ipv6/capability" {
            return Ok(json!({"multi_support":0,"multi_num":1,"bind_status":1}));
        }
        if path.starts_with(Resource::Ipv6Wan.path()) {
            let mut row = self.wan.read().await.clone();
            row["valid_lft"] = json!(1000 + self.leases.fetch_add(1, Ordering::SeqCst));
            return Ok(json!({"data":[row],"total":1}));
        }
        if path.starts_with(Resource::Ipv6Lan.path()) {
            return Ok(json!({"data":[self.lan.read().await.clone()],"total":1}));
        }
        if path.starts_with(Resource::LoadBalanceRules.path()) {
            return Ok(json!({"data":self.rows.read().await.clone(),"total":3}));
        }
        Err("Unknown model endpoint".into())
    }
    async fn put(&self, resource: Resource, body: &Value) -> Result<(), String> {
        self.calls.write().await.push(resource);
        match resource {
            Resource::Ipv6Wan => {
                assert!(body.get("dhcp6_ip_addr").is_none());
                assert!(body.get("dhcp6_prefix1").is_none());
                let interface = body["interface"].as_str().ok_or("Interface absent")?;
                let mut next = wan(interface, self.mode);
                for (key, value) in body.as_object().ok_or("Body invalid")? {
                    next[key] = value.clone();
                }
                *self.wan.write().await = next;
            }
            Resource::Ipv6Lan => {
                assert!(body.get("ipv6_addr").is_none());
                let parent = body["parent"].as_str().ok_or("Parent absent")?;
                if parent == "wan3" {
                    assert!(ikb_failover::ipv6::prefix(&*self.wan.read().await).is_some());
                }
                let mut next = lan(parent, self.mode);
                for (key, value) in body.as_object().ok_or("Body invalid")? {
                    next[key] = value.clone();
                }
                *self.lan.write().await = next;
            }
            Resource::LoadBalanceRules => {
                let mut rows = self.rows.write().await;
                *rows
                    .iter_mut()
                    .find(|row| row["id"] == body["id"])
                    .ok_or("Rule absent")? = body.clone();
            }
            _ => return Err("Unsupported model mutation".into()),
        }
        Ok(())
    }
}

fn config(directory: &std::path::Path) -> Config {
    let mut config: Config = serde_yaml::from_str(include_str!("fixtures/config.yml")).unwrap();
    config.management.apply = true;
    config.management.state_dir = directory.into();
    config
        .wan
        .rule_pairs
        .retain(|pair| pair.resource == Resource::LoadBalanceRules);
    config.wan.domain_rules.clear();
    config.wan.failure_threshold = 1;
    config.wan.recovery_threshold = 1;
    config.wan.stable_hold_ms = 0;
    let ipv6 = config.wan.ipv6.as_mut().unwrap();
    ipv6.settle_timeout_ms = 150;
    ipv6.poll_interval_ms = 10;
    config
}

#[tokio::test]
async fn primary_failure_migrates_single_slot_and_lan_then_recovery_fails_back() {
    let model = Arc::new(Model::new(Mode::Ready));
    let directory = tempfile::tempdir().unwrap();
    let runtime = Runtime::new(config(directory.path()), model.clone())
        .await
        .unwrap();
    runtime.cycle().await.unwrap();
    assert_eq!(runtime.snapshot().await["wan"]["state"]["active"], "backup");
    assert_eq!(model.wan.read().await["interface"], "wan3");
    assert_eq!(model.lan.read().await["parent"], "wan3");
    assert_eq!(model.rows.read().await[2]["interface"], "wan2");
    model.primary_up.store(true, Ordering::SeqCst);
    runtime.cycle().await.unwrap();
    assert_eq!(
        runtime.snapshot().await["wan"]["state"]["active"],
        "primary"
    );
    assert_eq!(model.wan.read().await["interface"], "wan1");
    assert_eq!(model.lan.read().await["parent"], "wan1");
    let journal = tokio::fs::read_to_string(directory.path().join("ipv6/journal.json"))
        .await
        .unwrap();
    assert!(!journal.contains("dhcp6_ip_addr"));
    assert!(!journal.contains("dhcp6_prefix1"));
    assert!(!journal.contains("2002:c000:0202"));
}

#[tokio::test]
async fn missing_or_stale_pd_rolls_back_ipv6_and_preserves_ipv4_failover() {
    for mode in [Mode::NoPrefix, Mode::StalePrefix, Mode::BadLan] {
        let model = Arc::new(Model::new(mode));
        let directory = tempfile::tempdir().unwrap();
        let config = config(directory.path());
        let runtime = Runtime::new(config.clone(), model.clone()).await.unwrap();
        tokio::time::timeout(Duration::from_secs(3), runtime.cycle())
            .await
            .unwrap()
            .unwrap();
        let status = runtime.snapshot().await;
        assert_eq!(status["wan"]["state"]["active"], "backup");
        assert_eq!(status["wan"]["ipv6_capability"], "degraded");
        assert!(!status["wan"]["ipv6_error"].as_str().unwrap().is_empty());
        assert!(status["wan"]["last_error"].as_str().unwrap().is_empty());
        assert_eq!(model.rows.read().await[0]["enabled"], "no");
        assert_eq!(model.rows.read().await[1]["enabled"], "yes");
        assert_eq!(model.rows.read().await[2]["interface"], "wan2");
        assert_eq!(model.wan.read().await["interface"], "wan1");
        assert_eq!(model.lan.read().await["parent"], "wan1");
        if matches!(mode, Mode::NoPrefix | Mode::StalePrefix) {
            assert!(!model.calls.read().await.contains(&Resource::Ipv6Lan));
        }
        model.primary_up.store(true, Ordering::SeqCst);
        runtime.cycle().await.unwrap();
        assert_eq!(
            runtime.snapshot().await["wan"]["state"]["active"],
            "primary"
        );
        assert_eq!(model.rows.read().await[0]["enabled"], "yes");
        assert_eq!(model.rows.read().await[1]["enabled"], "no");
    }
}

#[tokio::test]
async fn mixed_crash_journal_recovers_static_configuration_without_pinning_leases() {
    let model = Model::new(Mode::Ready);
    let directory = tempfile::tempdir().unwrap();
    let config = config(directory.path());
    let plan = policy::build(&model, &config.wan, Egress::Backup)
        .await
        .unwrap();
    assert!(plan.blockers.is_empty());
    assert_eq!(plan.ipv6_capability, "configuration_verified");
    let journal = Journal {
        phase: "prepared".into(),
        target: Egress::Backup,
        changed_ms: 1000,
        changes: plan.changes.clone(),
        error: String::new(),
    };
    tokio::fs::write(
        directory.path().join("journal.json"),
        serde_json::to_vec(&journal).unwrap(),
    )
    .await
    .unwrap();
    for change in plan.changes.iter().take(3) {
        model.put(change.resource, &change.after).await.unwrap();
    }
    let store = Store::open(directory.path()).await.unwrap();
    store.recover(&model, &config.wan).await.unwrap();
    assert_eq!(model.wan.read().await["interface"], "wan1");
    assert_eq!(model.lan.read().await["parent"], "wan1");
    assert_eq!(model.rows.read().await[0]["enabled"], "yes");
    assert_eq!(model.rows.read().await[1]["enabled"], "no");
    assert!(!store.has_pending().await.unwrap());
}

#[tokio::test]
async fn unchanged_ipv6_configuration_with_missing_pd_reports_degraded_ipv6() {
    let model = Arc::new(Model::new(Mode::Ready));
    model.primary_up.store(true, Ordering::SeqCst);
    model.wan.write().await["dhcp6_prefix1"] = json!("");
    let directory = tempfile::tempdir().unwrap();
    let runtime = Runtime::new(config(directory.path()), model.clone())
        .await
        .unwrap();
    runtime.cycle().await.unwrap();
    let status = runtime.snapshot().await;
    assert!(
        status["wan"]["plan"]["changes"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert!(
        status["wan"]["plan"]["blockers"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert_eq!(status["wan"]["ipv6_capability"], "degraded");
    assert!(!status["wan"]["ipv6_error"].as_str().unwrap().is_empty());
    assert_eq!(
        status["wan"]["plan"]["ipv6_evidence"]["delegated_prefix_ready"],
        false
    );
    assert!(model.calls.read().await.is_empty());
    assert!(directory.path().join("state.json").exists());
}

#[tokio::test]
async fn unfinished_ipv6_rollback_does_not_block_ipv4_failback() {
    let model = Arc::new(Model::new(Mode::Ready));
    let directory = tempfile::tempdir().unwrap();
    let settings = config(directory.path());
    let runtime = Runtime::new(settings.clone(), model.clone()).await.unwrap();
    runtime.cycle().await.unwrap();
    let mut plan = policy::build(model.as_ref(), &settings.wan, Egress::Primary)
        .await
        .unwrap();
    plan.changes.retain(|change| change.resource.is_ipv6());
    let journal = Journal {
        phase: "prepared".into(),
        target: Egress::Primary,
        changed_ms: 1000,
        changes: plan.changes.clone(),
        error: String::new(),
    };
    tokio::fs::write(
        directory.path().join("ipv6/journal.json"),
        serde_json::to_vec(&journal).unwrap(),
    )
    .await
    .unwrap();
    model
        .put(plan.changes[0].resource, &plan.changes[0].after)
        .await
        .unwrap();
    model.wan.write().await["force_gen_duid"] = json!(0);
    model.primary_up.store(true, Ordering::SeqCst);
    runtime.cycle().await.unwrap();
    let status = runtime.snapshot().await;
    assert_eq!(status["wan"]["state"]["active"], "primary");
    assert_eq!(status["wan"]["ipv6_capability"], "degraded");
    assert_eq!(model.rows.read().await[0]["enabled"], "yes");
    assert_eq!(model.rows.read().await[1]["enabled"], "no");
    assert_eq!(model.wan.read().await["force_gen_duid"], 0);
}

#[tokio::test]
async fn production_ipv4_loop_fails_back_while_ipv6_pd_wait_is_still_running() {
    let model = Arc::new(Model::new(Mode::NoPrefix));
    let directory = tempfile::tempdir().unwrap();
    let mut settings = config(directory.path());
    settings.management.listen = "127.0.0.1:0".parse().unwrap();
    settings.wan.check_interval_ms = 100;
    settings.wan.ipv6.as_mut().unwrap().settle_timeout_ms = 800;
    let runtime = Arc::new(Runtime::new(settings, model.clone()).await.unwrap());
    let cancel = tokio_util::sync::CancellationToken::new();
    let daemon = {
        let runtime = runtime.clone();
        let cancel = cancel.clone();
        tokio::spawn(async move {
            tokio::time::timeout(Duration::from_secs(5), runtime.run(cancel))
                .await
                .unwrap()
                .unwrap();
        })
    };
    tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            if model.calls.read().await.contains(&Resource::Ipv6Wan) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    model.primary_up.store(true, Ordering::SeqCst);
    tokio::time::timeout(Duration::from_millis(400), async {
        loop {
            if runtime.snapshot().await["wan"]["state"]["active"] == "primary" {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(model.rows.read().await[0]["enabled"], "yes");
    assert_eq!(model.rows.read().await[1]["enabled"], "no");
    cancel.cancel();
    daemon.await.unwrap();
}
