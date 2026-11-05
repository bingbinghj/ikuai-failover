use std::{
    fs::{File, Permissions},
    io::ErrorKind,
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use base64::Engine;
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use tokio::io::AsyncWriteExt;

use crate::policy::Resource;

const DEFAULT_CONFIG: &str = include_str!("../../../config.example.yml");
static TEMPORARY_SEQUENCE: AtomicU64 = AtomicU64::new(0);
pub const CONFIG_FILE_CHANGED: &str = "Configuration changed; reload settings before saving";

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(skip)]
    pub source_path: PathBuf,
    pub ikuai: IkuaiConfig,
    pub wan: WanConfig,
    pub management: ManagementConfig,
    #[serde(default)]
    pub notifications: crate::notification::NotificationConfig,
    #[serde(default)]
    pub webui: crate::auth::WebUiAuthConfig,
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IkuaiConfig {
    pub base_url: String,
    #[serde(default)]
    pub username: String,
    #[serde(default)]
    pub password: String,
    pub allow_invalid_certificate: bool,
    pub timeout_ms: u64,
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManagementConfig {
    pub listen: SocketAddr,
    pub apply: bool,
    pub state_dir: PathBuf,
    #[serde(default = "default_web_dir")]
    pub web_dir: PathBuf,
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WanConfig {
    pub enabled: bool,
    pub primary: String,
    pub backup: String,
    pub preserved_default: String,
    pub check_interval_ms: u64,
    pub failure_threshold: u32,
    pub recovery_threshold: u32,
    pub stable_hold_ms: u64,
    pub ipv6_enabled: bool,
    pub rule_pairs: Vec<RulePair>,
    #[serde(default)]
    pub ipv6: Option<Ipv6Config>,
    #[serde(default)]
    pub domain_rules: Vec<DomainBinding>,
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DomainBinding {
    pub id: i64,
    pub tag: String,
    pub source: String,
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Ipv6Config {
    pub wan_id: i64,
    pub wan_tag: String,
    pub lan_id: i64,
    pub lan_tag: String,
    pub lan_interface: String,
    pub settle_timeout_ms: u64,
    pub poll_interval_ms: u64,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RulePair {
    pub resource: Resource,
    pub primary_id: i64,
    pub backup_id: i64,
    pub primary_tag: String,
    pub backup_tag: String,
}

pub async fn load(path: &Path) -> Result<Config, String> {
    let text = match tokio::fs::read_to_string(path).await {
        Ok(text) => text,
        Err(error) if error.kind() == ErrorKind::NotFound => {
            initialize(path).await?;
            tokio::fs::read_to_string(path)
                .await
                .map_err(|error| format!("read config {}: {error}", path.display()))?
        }
        Err(error) => return Err(format!("read config {}: {error}", path.display())),
    };
    let source_path = tokio::fs::canonicalize(path)
        .await
        .map_err(|error| format!("resolve config {}: {error}", path.display()))?;
    let mut config: Config =
        serde_yaml::from_str(&text).map_err(|e| format!("parse config: {e}"))?;
    config.ikuai.apply_environment();
    config.webui.apply_environment();
    if config.webui.user.is_empty() || config.webui.pass.is_empty() {
        let mut generated = false;
        let document = update_document(&source_path, None, |document| {
            let mut current: Config = serde_yaml::from_value(document.clone())
                .map_err(|_| "The configuration no longer has valid settings".to_string())?;
            current.webui.apply_environment();
            if current.webui.user.is_empty() || current.webui.pass.is_empty() {
                let mut random = [0u8; 32];
                getrandom::fill(&mut random)
                    .map_err(|_| "Could not generate Web console credentials".to_string())?;
                if current.webui.user.is_empty() {
                    let suffix: String = random[..8]
                        .iter()
                        .map(|byte| format!("{byte:02x}"))
                        .collect();
                    current.webui.user = format!("user_{suffix}");
                }
                if current.webui.pass.is_empty() {
                    current.webui.pass =
                        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(&random[8..]);
                }
                let root = document
                    .as_mapping_mut()
                    .ok_or_else(|| "The configuration must be a YAML object".to_string())?;
                let key = serde_yaml::Value::String("webui".into());
                if !root.contains_key(&key) {
                    root.insert(key.clone(), serde_yaml::Value::Mapping(Default::default()));
                }
                let webui = root
                    .get_mut(&key)
                    .and_then(serde_yaml::Value::as_mapping_mut)
                    .ok_or_else(|| {
                        "The Web console configuration must be a YAML object".to_string()
                    })?;
                webui.insert(
                    serde_yaml::Value::String("user".into()),
                    serde_yaml::Value::String(current.webui.user),
                );
                webui.insert(
                    serde_yaml::Value::String("pass".into()),
                    serde_yaml::Value::String(current.webui.pass),
                );
                generated = true;
            }
            Ok(())
        })
        .await?;
        if generated {
            crate::logging::info(&format!(
                "[网页登录配置已保存] 请在 {} 查看 webui.user 和 webui.pass。",
                source_path.display()
            ));
        }
        config = serde_yaml::from_value(document)
            .map_err(|_| "The configuration no longer has valid settings".to_string())?;
        config.ikuai.apply_environment();
        config.webui.apply_environment();
    }
    config.source_path = source_path;
    validate(&config)?;
    Ok(config)
}

pub fn validate_candidate(document: &serde_yaml::Value) -> Result<Config, String> {
    let config: Config = serde_yaml::from_value(document.clone())
        .map_err(|error| format!("parse config: {error}"))?;
    validate(&config)?;
    Ok(config)
}

pub fn validate(config: &Config) -> Result<(), String> {
    config.webui.validate()?;
    config
        .notifications
        .validate()
        .map_err(|e| format!("Invalid notification configuration: {e}"))?;
    if !is_wan_interface(&config.wan.primary)
        || !is_wan_interface(&config.wan.backup)
        || !is_wan_interface(&config.wan.preserved_default)
        || config.wan.primary == config.wan.backup
        || config.wan.primary == config.wan.preserved_default
        || config.wan.backup == config.wan.preserved_default
    {
        return Err("Select distinct primary, backup, and preserved default WAN interfaces".into());
    }
    if config.wan.check_interval_ms < 100
        || config.wan.failure_threshold == 0
        || config.wan.recovery_threshold == 0
        || config.ikuai.timeout_ms == 0
        || config.ikuai.timeout_ms > 30000
    {
        return Err("Invalid polling or threshold configuration".into());
    }
    if !config.ikuai.base_url.starts_with("https://") {
        return Err("An HTTPS iKuai URL is required".into());
    }
    let mut ids = std::collections::HashSet::new();
    for pair in &config.wan.rule_pairs {
        if !matches!(
            pair.resource,
            Resource::FiveTupleRules | Resource::LoadBalanceRules | Resource::DomainRuleState
        ) || pair.primary_id <= 0
            || pair.backup_id <= 0
            || pair.primary_id == pair.backup_id
            || pair.primary_tag.is_empty()
            || pair.backup_tag.is_empty()
            || !ids.insert((pair.resource.path(), pair.primary_id))
            || !ids.insert((pair.resource.path(), pair.backup_id))
        {
            return Err("Invalid or overlapping explicit rule pair".into());
        }
    }
    if let Some(ipv6) = &config.wan.ipv6 {
        if ipv6.wan_id <= 0
            || ipv6.lan_id <= 0
            || ipv6.wan_tag.is_empty()
            || ipv6.lan_tag.is_empty()
            || ipv6.lan_interface.is_empty()
            || !(100..=90000).contains(&ipv6.settle_timeout_ms)
            || !(10..=5000).contains(&ipv6.poll_interval_ms)
        {
            return Err("Invalid explicit IPv6 profile or settlement timeout".into());
        }
    }
    for binding in &config.wan.domain_rules {
        if binding.id <= 0
            || binding.tag.is_empty()
            || binding.source.is_empty()
            || !ids.insert((Resource::DomainRules.path(), binding.id))
        {
            return Err("Invalid explicit domain-rule binding".into());
        }
    }
    Ok(())
}

pub fn is_wan_interface(name: &str) -> bool {
    name.strip_prefix("wan").is_some_and(|number| {
        !number.is_empty()
            && number.len() <= 5
            && number.bytes().all(|byte| byte.is_ascii_digit())
            && !number.starts_with('0')
    })
}

async fn initialize(path: &Path) -> Result<(), String> {
    let temporary = write_temporary(path, DEFAULT_CONFIG.as_bytes(), None).await?;
    let published = tokio::fs::hard_link(&temporary, path).await;
    let _ = tokio::fs::remove_file(&temporary).await;
    match published {
        Ok(()) => {
            crate::logging::info(&format!("[配置已生成] {}", path.display()));
            Ok(())
        }
        // Linking a complete file publishes atomically and never replaces an existing config.
        Err(error) if error.kind() == ErrorKind::AlreadyExists => Ok(()),
        Err(error) => Err(initialization_error(path, error)),
    }
}

async fn write_temporary(
    path: &Path,
    contents: &[u8],
    permissions: Option<&Permissions>,
) -> Result<PathBuf, String> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let filename = path
        .file_name()
        .ok_or_else(|| "The configuration path must name a file".to_string())?;
    tokio::fs::create_dir_all(parent)
        .await
        .map_err(|error| initialization_error(path, error))?;
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    let mut reserved = None;
    for _ in 0..16 {
        let sequence = TEMPORARY_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let mut temporary_name = filename.to_os_string();
        temporary_name.push(format!(
            ".bootstrap-{}-{timestamp}-{sequence}.tmp",
            std::process::id()
        ));
        let temporary = parent.join(temporary_name);
        let mut options = tokio::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            // New files inherit the directory ACL and umask from a 0666 creation mode.
            options.mode(permissions.map_or(0o666, |permissions| permissions.mode()));
        }
        match options.open(&temporary).await {
            Ok(file) => {
                reserved = Some((temporary, file));
                break;
            }
            Err(error) if error.kind() == ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(initialization_error(path, error)),
        }
    }
    let (temporary, mut file) =
        reserved.ok_or_else(|| "Could not reserve an initial configuration file".to_string())?;
    let prepared = async {
        file.write_all(contents).await?;
        if let Some(permissions) = permissions {
            if file.metadata().await?.permissions() != *permissions {
                file.set_permissions(permissions.clone()).await?;
            }
        }
        file.sync_all().await
    }
    .await;
    drop(file);
    if let Err(error) = prepared {
        let _ = tokio::fs::remove_file(&temporary).await;
        return Err(initialization_error(path, error));
    }
    Ok(temporary)
}

async fn configuration_lock(path: &Path) -> Result<File, String> {
    let mut lock_name = path
        .file_name()
        .ok_or_else(|| "The configuration path must name a file".to_string())?
        .to_os_string();
    lock_name.push(".lock");
    let lock_path = path.with_file_name(lock_name);
    tokio::task::spawn_blocking(move || {
        let mut options = std::fs::OpenOptions::new();
        options.read(true).write(true).create(true).truncate(false);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let lock = options.open(lock_path).map_err(|error| {
            format!(
                "Cannot lock configuration: {error}; mount the configuration directory as writable"
            )
        })?;
        FileExt::try_lock_exclusive(&lock)
            .map_err(|_| "Another process is updating the configuration".to_string())?;
        Ok(lock)
    })
    .await
    .map_err(|_| "The configuration lock is unavailable".to_string())?
}

/// Serialize cooperating writers and reject stale API updates before replacing the file.
pub async fn update_document<F>(
    path: &Path,
    expected: Option<&[u8]>,
    update: F,
) -> Result<serde_yaml::Value, String>
where
    F: FnOnce(&mut serde_yaml::Value) -> Result<(), String> + Send,
{
    let _lock = configuration_lock(path).await?;
    let current = tokio::fs::read(path)
        .await
        .map_err(|error| format!("read config {}: {error}", path.display()))?;
    if expected.is_some_and(|expected| expected != current.as_slice()) {
        return Err(CONFIG_FILE_CHANGED.into());
    }
    let mut document: serde_yaml::Value = serde_yaml::from_slice(&current)
        .map_err(|_| "The configuration is not a valid YAML document".to_string())?;
    let before = document.clone();
    update(&mut document)?;
    if document == before {
        return Ok(document);
    }
    let contents = serde_yaml::to_string(&document)
        .map_err(|_| "The configuration could not be encoded".to_string())?;
    save_contents(path, &current, contents.as_bytes()).await?;
    Ok(document)
}

/// Save the supplied text verbatim after checking the revision under the configuration lock.
pub async fn replace_document(path: &Path, expected: &[u8], contents: &str) -> Result<(), String> {
    let _lock = configuration_lock(path).await?;
    let current = tokio::fs::read(path)
        .await
        .map_err(|error| format!("read config {}: {error}", path.display()))?;
    if expected != current.as_slice() {
        return Err(CONFIG_FILE_CHANGED.into());
    }
    save_contents(path, &current, contents.as_bytes()).await
}

// Callers hold the configuration lock while preparing and publishing the replacement.
async fn save_contents(path: &Path, current: &[u8], contents: &[u8]) -> Result<(), String> {
    if contents == current {
        return Ok(());
    }
    let permissions = tokio::fs::metadata(path)
        .await
        .map_err(|error| format!("read config permissions {}: {error}", path.display()))?
        .permissions();
    let temporary = write_temporary(path, contents, Some(&permissions)).await?;
    let latest = tokio::fs::read(path).await;
    match latest {
        Ok(latest) if latest.as_slice() != current => {
            let _ = tokio::fs::remove_file(&temporary).await;
            return Err(CONFIG_FILE_CHANGED.into());
        }
        Err(error) => {
            let _ = tokio::fs::remove_file(&temporary).await;
            return Err(format!("read config {}: {error}", path.display()));
        }
        _ => {}
    }
    match tokio::fs::metadata(path).await {
        Ok(latest) if latest.permissions() != permissions => {
            let _ = tokio::fs::remove_file(&temporary).await;
            return Err(CONFIG_FILE_CHANGED.into());
        }
        Err(error) => {
            let _ = tokio::fs::remove_file(&temporary).await;
            return Err(format!(
                "read config permissions {}: {error}",
                path.display()
            ));
        }
        _ => {}
    }
    if let Err(error) = tokio::fs::rename(&temporary, path).await {
        let _ = tokio::fs::remove_file(&temporary).await;
        return Err(format!(
            "Cannot save config {}: {error}; mount the configuration directory as writable",
            path.display()
        ));
    }
    Ok(())
}

fn initialization_error(path: &Path, error: std::io::Error) -> String {
    format!(
        "Cannot initialize config {}: {error}; mount the configuration directory as writable",
        path.display()
    )
}

impl IkuaiConfig {
    pub fn has_credentials(&self) -> bool {
        !self.username.is_empty() && !self.password.is_empty()
    }

    pub fn apply_environment(&mut self) {
        for (key, field) in [
            ("IKUAI_FAILOVER_USERNAME", &mut self.username),
            ("IKUAI_FAILOVER_PASSWORD", &mut self.password),
        ] {
            if let Some(value) = optional_secret(&Some(key.to_string())) {
                *field = value;
            }
        }
    }
}

pub fn optional_secret(key: &Option<String>) -> Option<String> {
    key.as_ref()
        .and_then(|key| std::env::var(key).ok())
        .filter(|value| !value.is_empty())
}

impl WanConfig {
    pub fn interval(&self) -> Duration {
        Duration::from_millis(self.check_interval_ms)
    }
}

fn default_web_dir() -> PathBuf {
    PathBuf::from("/usr/share/ikuai-failover/web")
}
