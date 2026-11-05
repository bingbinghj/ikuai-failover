use std::{
    fs::File,
    path::{Path, PathBuf},
};

use fs2::FileExt;
use serde::{Deserialize, Serialize};
use tokio::io::AsyncWriteExt;

use crate::{
    api::RouterApi,
    config::WanConfig,
    policy::{self, Change, Plan},
    state::{Egress, WanState},
};

pub struct Store {
    directory: PathBuf,
    lock: File,
}

#[derive(Deserialize, Serialize)]
pub struct Journal {
    pub phase: String,
    pub target: Egress,
    pub changed_ms: u64,
    pub changes: Vec<Change>,
    pub error: String,
}

fn validate(journal: &Journal, config: &WanConfig) -> Result<(), String> {
    for change in &journal.changes {
        let mut known = false;
        for pair in &config.rule_pairs {
            if pair.resource != change.resource {
                continue;
            }
            for (id, tag, interface) in [
                (
                    pair.primary_id,
                    pair.primary_tag.as_str(),
                    config.primary.as_str(),
                ),
                (
                    pair.backup_id,
                    pair.backup_tag.as_str(),
                    config.backup.as_str(),
                ),
            ] {
                if id == change.id
                    && change
                        .before
                        .get("tagname")
                        .and_then(serde_json::Value::as_str)
                        == Some(tag)
                    && change
                        .before
                        .get("interface")
                        .and_then(serde_json::Value::as_str)
                        == Some(interface)
                {
                    known = true;
                }
            }
        }
        if let Some(ipv6) = &config.ipv6 {
            known |= match change.resource {
                policy::Resource::Ipv6Wan => {
                    change.id == ipv6.wan_id
                        && change
                            .before
                            .get("tagname")
                            .and_then(serde_json::Value::as_str)
                            == Some(ipv6.wan_tag.as_str())
                        && [config.primary.as_str(), config.backup.as_str()].contains(
                            &change
                                .before
                                .get("interface")
                                .and_then(serde_json::Value::as_str)
                                .unwrap_or_default(),
                        )
                }
                policy::Resource::Ipv6Lan => {
                    change.id == ipv6.lan_id
                        && change
                            .before
                            .get("tagname")
                            .and_then(serde_json::Value::as_str)
                            == Some(ipv6.lan_tag.as_str())
                        && change
                            .before
                            .get("interface")
                            .and_then(serde_json::Value::as_str)
                            == Some(ipv6.lan_interface.as_str())
                        && [config.primary.as_str(), config.backup.as_str()].contains(
                            &change
                                .before
                                .get("parent")
                                .and_then(serde_json::Value::as_str)
                                .unwrap_or_default(),
                        )
                }
                _ => false,
            };
        }
        if change.resource == policy::Resource::DomainRules {
            known |= config.domain_rules.iter().any(|binding| {
                binding.id == change.id
                    && change
                        .before
                        .get("tagname")
                        .and_then(serde_json::Value::as_str)
                        == Some(binding.tag.as_str())
                    && change
                        .before
                        .get("enabled")
                        .and_then(serde_json::Value::as_str)
                        == Some("yes")
                    && policy::source_matches(&change.before, &binding.source)
                    && policy::physical_interface(
                        change
                            .before
                            .get("interface")
                            .and_then(serde_json::Value::as_str)
                            .unwrap_or_default(),
                        config,
                    )
            });
        }
        let normalized = policy::normalized(change.resource, &change.before)?;
        let mut expected = normalized.clone();
        let field = change.resource.mutation_field();
        expected[field] = change
            .after
            .get(field)
            .cloned()
            .ok_or_else(|| "Journal changed field is missing".to_string())?;
        let allowed = if change.resource == policy::Resource::DomainRules {
            policy::physical_interface(
                change
                    .after
                    .get(field)
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default(),
                config,
            )
        } else if change.resource.is_ipv6() {
            [config.primary.as_str(), config.backup.as_str()].contains(
                &change
                    .after
                    .get(field)
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default(),
            )
        } else {
            matches!(
                change.after.get(field).and_then(serde_json::Value::as_str),
                Some("yes" | "no")
            )
        };
        if !known
            || normalized != change.before
            || expected != change.after
            || change.before.get("id").and_then(serde_json::Value::as_i64) != Some(change.id)
            || !allowed
        {
            return Err("Journal changes no longer match the explicit rule/field allowlist".into());
        }
    }
    Ok(())
}

async fn write<T: Serialize>(path: &Path, value: &T) -> Result<(), String> {
    let bytes = serde_json::to_vec_pretty(value).map_err(|e| e.to_string())?;
    let temporary = path.with_extension("tmp");
    let mut file = tokio::fs::File::create(&temporary)
        .await
        .map_err(|e| e.to_string())?;
    file.write_all(&bytes).await.map_err(|e| e.to_string())?;
    file.sync_all().await.map_err(|e| e.to_string())?;
    drop(file);
    tokio::fs::rename(temporary, path)
        .await
        .map_err(|e| e.to_string())
}

impl Store {
    pub async fn open(directory: &Path) -> Result<Self, String> {
        tokio::fs::create_dir_all(directory)
            .await
            .map_err(|e| e.to_string())?;
        let lock_path = directory.join("daemon.lock");
        let lock = tokio::task::spawn_blocking(move || {
            let lock = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(false)
                .open(lock_path)?;
            lock.try_lock_exclusive()?;
            Ok::<File, std::io::Error>(lock)
        })
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| format!("Another daemon may own the state directory: {e}"))?;
        Ok(Self {
            directory: directory.to_path_buf(),
            lock,
        })
    }

    pub async fn state(&self) -> Result<WanState, String> {
        let mut state = match tokio::fs::read(self.directory.join("state.json")).await {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(|e| e.to_string()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(WanState::default()),
            Err(error) => Err(error.to_string()),
        }?;
        if let Some(journal) = self.read_journal().await? {
            if journal.phase == "committed" && journal.changed_ms >= state.last_transition_ms {
                state.commit(journal.target, journal.changed_ms);
            }
        }
        Ok(state)
    }
    pub async fn save_state(&self, state: &WanState) -> Result<(), String> {
        write(&self.directory.join("state.json"), state).await
    }

    async fn read_journal(&self) -> Result<Option<Journal>, String> {
        let bytes = match tokio::fs::read(self.directory.join("journal.json")).await {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.to_string()),
        };
        serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|e| e.to_string())
    }

    pub async fn has_pending(&self) -> Result<bool, String> {
        Ok(self.read_journal().await?.is_some_and(|journal| {
            journal.phase == "prepared" || journal.phase == "rollback_blocked"
        }))
    }

    pub async fn recover(&self, api: &dyn RouterApi, config: &WanConfig) -> Result<(), String> {
        let Some(mut journal) = self.read_journal().await? else {
            return Ok(());
        };
        if journal.phase == "prepared" || journal.phase == "rollback_blocked" {
            validate(&journal, config)?;
            self.rollback(api, &mut journal).await?;
        }
        Ok(())
    }

    async fn rollback(&self, api: &dyn RouterApi, journal: &mut Journal) -> Result<(), String> {
        for change in journal.changes.iter().rev() {
            let current = policy::current(api, change.resource, change.id).await?;
            if current == change.before {
                continue;
            }
            if current != change.after {
                journal.phase = "rollback_blocked".into();
                journal.error = "External changes prevent rollback; manual review required".into();
                write(&self.directory.join("journal.json"), journal).await?;
                return Err(journal.error.clone());
            }
            api.put(change.resource, &change.before).await?;
            if policy::current(api, change.resource, change.id).await? != change.before {
                return Err("Rollback readback mismatch".into());
            }
        }
        journal.phase = "rolled_back".into();
        write(&self.directory.join("journal.json"), journal).await
    }

    pub async fn apply(
        &self,
        api: &dyn RouterApi,
        plan: &Plan,
        config: &WanConfig,
        now: u64,
    ) -> Result<(), String> {
        if !plan.blockers.is_empty() {
            return Err(plan.blockers.join("; "));
        }
        if config.ipv6_enabled && !plan.ipv6_error.is_empty() {
            return Err(plan.ipv6_error.clone());
        }
        if self.has_pending().await? {
            return Err("Unfinished journal must be recovered before another transaction".into());
        }
        let mut journal = Journal {
            phase: "prepared".into(),
            target: plan.target,
            changed_ms: now,
            changes: plan.changes.clone(),
            error: String::new(),
        };
        validate(&journal, config)?;
        write(&self.directory.join("journal.json"), &journal).await?;
        let result = async {
            let deadline = config.ipv6.as_ref().map(|ipv6| {
                tokio::time::Instant::now()
                    + std::time::Duration::from_millis(ipv6.settle_timeout_ms)
            });
            for change in &journal.changes {
                let current = policy::current(api, change.resource, change.id).await?;
                let previous =
                    if change.resource == policy::Resource::Ipv6Wan && current != change.after {
                        let settings = config
                            .ipv6
                            .as_ref()
                            .ok_or_else(|| "Missing IPv6 settlement profile".to_string())?;
                        Some(crate::ipv6::lease(
                            &crate::ipv6::raw_wan(api, settings).await?,
                        ))
                    } else {
                        None
                    };
                if current != change.after {
                    if current != change.before {
                        return Err("Snapshot changed before write".to_string());
                    }
                    api.put(change.resource, &change.after).await?;
                }
                if policy::current(api, change.resource, change.id).await? != change.after {
                    return Err("Policy readback mismatch".to_string());
                }
                if change.resource.is_ipv6() {
                    let settings = config
                        .ipv6
                        .as_ref()
                        .ok_or_else(|| "Missing IPv6 settlement profile".to_string())?;
                    let deadline =
                        deadline.ok_or_else(|| "Missing IPv6 settlement deadline".to_string())?;
                    let interface = if plan.target == Egress::Primary {
                        &config.primary
                    } else {
                        &config.backup
                    };
                    if change.resource == policy::Resource::Ipv6Wan {
                        crate::ipv6::wait_wan(api, settings, interface, previous, deadline).await?;
                    } else {
                        crate::ipv6::wait_lan(api, settings, interface, deadline).await?;
                    }
                }
            }
            if config.ipv6_enabled {
                let settings = config
                    .ipv6
                    .as_ref()
                    .ok_or_else(|| "Missing IPv6 settlement profile".to_string())?;
                let interface = if plan.target == Egress::Primary {
                    &config.primary
                } else {
                    &config.backup
                };
                let deadline =
                    deadline.ok_or_else(|| "Missing IPv6 settlement deadline".to_string())?;
                crate::ipv6::wait_wan(api, settings, interface, None, deadline).await?;
                crate::ipv6::wait_lan(api, settings, interface, deadline).await?;
            }
            Ok(())
        }
        .await;
        if let Err(error) = result {
            journal.error = error;
            let original = journal.error.clone();
            self.rollback(api, &mut journal)
                .await
                .map_err(|rollback| format!("{original}; rollback: {rollback}"))?;
            return Err(original);
        }
        journal.phase = "committed".into();
        write(&self.directory.join("journal.json"), &journal).await
    }
}

impl Drop for Store {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self.lock);
    }
}
