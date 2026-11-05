use std::{collections::HashMap, sync::Arc, time::Duration};

use futures_util::{StreamExt, stream};
use serde::{Deserialize, Serialize};

use super::transport::PreparedChannel;
use super::{ChannelConfig, NotificationConfig, NotificationError, providers, transport};

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Event {
    pub id: String,
    pub kind: String,
    pub title: String,
    pub text: String,
    pub timestamp_ms: u64,
    #[serde(default)]
    pub context: super::network_context::NetworkContext,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous_context: Option<super::network_context::NetworkContext>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct DeliveryResult {
    pub tag: String,
    pub provider: String,
    pub success: bool,
    pub error_code: Option<String>,
}

pub struct Notifier {
    client: reqwest::Client,
    timeout: Duration,
    channels: Vec<Arc<ChannelEntry>>,
    templates: std::collections::BTreeMap<String, String>,
}

struct ChannelEntry {
    tag: String,
    provider: String,
    prepared: Result<PreparedChannel, NotificationError>,
}

impl Notifier {
    pub fn new(config: &NotificationConfig) -> Result<Self, NotificationError> {
        Self::new_with_env(config, transport::secret_env)
    }

    pub fn new_with_env(
        config: &NotificationConfig,
        read_env: impl Fn(&str) -> Result<String, NotificationError>,
    ) -> Result<Self, NotificationError> {
        config.validate()?;
        let read_secret = |key: &str| {
            read_env(key).or_else(|_| {
                config
                    .secrets
                    .get(key)
                    .filter(|value| !value.trim().is_empty())
                    .cloned()
                    .ok_or(NotificationError::SecretMissing)
            })
        };
        let channels = if config.enabled {
            config
                .channels
                .iter()
                .filter(|channel| channel.enabled)
                .map(|channel| {
                    Arc::new(ChannelEntry {
                        tag: channel.tag.clone(),
                        provider: channel.provider.name().into(),
                        prepared: prepare(channel, &read_secret),
                    })
                })
                .collect()
        } else {
            Vec::new()
        };
        Ok(Self {
            client: transport::client(config.timeout_ms)?,
            timeout: Duration::from_millis(config.timeout_ms),
            channels,
            templates: config.templates.clone(),
        })
    }

    pub async fn send(&self, event: &Event) -> Vec<DeliveryResult> {
        self.send_to(event, &[]).await
    }

    pub async fn send_to(&self, event: &Event, tags: &[String]) -> Vec<DeliveryResult> {
        let rendered = super::templates::render(
            &super::templates::select(&self.templates, &event.kind),
            event,
        );
        let channels = self
            .channels
            .iter()
            .filter(|channel| tags.is_empty() || tags.contains(&channel.tag))
            .cloned()
            .collect::<Vec<_>>();
        let rendered = &rendered;
        stream::iter(channels)
            .map(|channel: Arc<ChannelEntry>| async move {
                let result = match (&channel.prepared, &rendered) {
                    (Ok(prepared), Ok(event)) => match tokio::time::timeout(
                        self.timeout,
                        providers::send(&self.client, prepared, event),
                    )
                    .await
                    {
                        Ok(result) => result,
                        Err(_) => Err(NotificationError::Timeout),
                    },
                    (Err(error), _) | (_, Err(error)) => Err(*error),
                };
                DeliveryResult {
                    tag: channel.tag.clone(),
                    provider: channel.provider.clone(),
                    success: result.is_ok(),
                    error_code: result.err().map(|error| error.to_string()),
                }
            })
            .buffer_unordered(8)
            .collect()
            .await
    }
}

fn prepare(
    channel: &ChannelConfig,
    read_env: &impl Fn(&str) -> Result<String, NotificationError>,
) -> Result<PreparedChannel, NotificationError> {
    let mut secrets = HashMap::new();
    let message_format = super::formats::effective(channel);
    if channel
        .mp_config
        .get("WECHAT_MODE")
        .and_then(serde_json::Value::as_str)
        == Some("bot")
    {
        return Ok(PreparedChannel {
            provider: channel.provider.clone(),
            secrets,
            mp_config: channel.mp_config.clone(),
            message_format,
        });
    }
    for key in channel.provider.required_env() {
        let value = read_env(key)?;
        if value.trim().is_empty() {
            return Err(NotificationError::SecretMissing);
        }
        secrets.insert(key.to_owned(), value);
    }
    for key in channel.provider.optional_env() {
        if let Ok(value) = read_env(key) {
            if !value.trim().is_empty() {
                secrets.insert(key.to_owned(), value);
            }
        }
    }
    Ok(PreparedChannel {
        provider: channel.provider.clone(),
        secrets,
        mp_config: channel.mp_config.clone(),
        message_format,
    })
}
