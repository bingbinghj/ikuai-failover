use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::config::WanConfig;

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Egress {
    #[default]
    Primary,
    Backup,
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Health {
    Up,
    Down,
    Unknown,
}

#[derive(Clone, Default, Deserialize, Serialize)]
pub struct WanState {
    pub active: Egress,
    pub failed: u32,
    pub recovered: u32,
    pub last_transition_ms: u64,
}

pub fn health(results: &Value, interface: &str) -> Health {
    let Some(rows) = results.get("iface_check").and_then(Value::as_array) else {
        return Health::Unknown;
    };
    let Some(row) = rows
        .iter()
        .find(|row| row.get("interface").and_then(Value::as_str) == Some(interface))
    else {
        return Health::Unknown;
    };
    match row.get("result").and_then(Value::as_str) {
        Some("success") => Health::Up,
        Some("fail" | "failed" | "failure" | "error") => Health::Down,
        _ => Health::Unknown,
    }
}

impl WanState {
    pub fn observe(
        &mut self,
        primary: Health,
        backup: Health,
        config: &WanConfig,
        now: u64,
    ) -> Egress {
        match primary {
            Health::Up => {
                self.recovered = self.recovered.saturating_add(1);
                self.failed = 0;
            }
            Health::Down => {
                self.failed = self.failed.saturating_add(1);
                self.recovered = 0;
            }
            Health::Unknown => {
                self.failed = 0;
                self.recovered = 0;
            }
        }
        if now.saturating_sub(self.last_transition_ms) < config.stable_hold_ms {
            return self.active;
        }
        match self.active {
            Egress::Primary if self.failed >= config.failure_threshold && backup == Health::Up => {
                Egress::Backup
            }
            Egress::Backup if self.recovered >= config.recovery_threshold => Egress::Primary,
            _ => self.active,
        }
    }

    pub fn commit(&mut self, target: Egress, now: u64) {
        if target != self.active {
            self.active = target;
            self.last_transition_ms = now;
            self.failed = 0;
            self.recovered = 0;
        }
    }
}
