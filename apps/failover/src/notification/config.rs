use std::collections::{BTreeMap, HashSet};

use serde::{Deserialize, Serialize};

use super::NotificationError;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct NotificationConfig {
    pub enabled: bool,
    pub timeout_ms: u64,
    pub events: Vec<String>,
    pub cooldown_ms: u64,
    pub queue_capacity: usize,
    pub channels: Vec<ChannelConfig>,
    pub secrets: BTreeMap<String, String>,
    pub templates: BTreeMap<String, String>,
}

impl Default for NotificationConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            timeout_ms: 10_000,
            events: Vec::new(),
            cooldown_ms: 60_000,
            queue_capacity: 128,
            channels: Vec::new(),
            secrets: BTreeMap::new(),
            templates: super::templates::configured_defaults(),
        }
    }
}

impl NotificationConfig {
    pub fn validate(&self) -> Result<(), NotificationError> {
        if self.templates.len() > 64 {
            return Err(NotificationError::InvalidConfiguration);
        }
        for (kind, template) in &self.templates {
            if kind.is_empty() || kind.len() > 64 {
                return Err(NotificationError::InvalidConfiguration);
            }
            super::templates::validate(template)?;
        }
        if !(100..=60_000).contains(&self.timeout_ms) || !(1..=4096).contains(&self.queue_capacity)
        {
            return Err(NotificationError::InvalidConfiguration);
        }
        if self.channels.len() > 64 {
            return Err(NotificationError::InvalidConfiguration);
        }
        if self.secrets.len() > 128
            || self.secrets.iter().any(|(key, value)| {
                key.is_empty()
                    || key.len() > 128
                    || !key
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
                    || value.len() > 1_048_576
            })
        {
            return Err(NotificationError::InvalidConfiguration);
        }
        let mut tags = HashSet::new();
        for channel in &self.channels {
            if ["WECHAT_APP_MARKDOWN", "QQ_MARKDOWN"].iter().any(|key| {
                channel
                    .mp_config
                    .get(*key)
                    .is_some_and(|value| !value.is_boolean())
            }) {
                return Err(NotificationError::InvalidConfiguration);
            }
            if channel
                .message_format
                .as_deref()
                .is_some_and(|value| !super::formats::valid(channel, value))
            {
                return Err(NotificationError::InvalidConfiguration);
            }
            if channel.name.len() > 256
                || channel.mp_config.len() > 128
                || serde_json::to_vec(&channel.mp_config)
                    .map_or(true, |bytes| bytes.len() > 256 * 1024)
                || channel.switchs.len() > super::event_types::EVENTS.len()
                || channel
                    .switchs
                    .iter()
                    .any(|kind| !super::event_types::known(kind))
            {
                return Err(NotificationError::InvalidConfiguration);
            }
            if channel.tag.is_empty()
                || channel.tag.len() > 64
                || !channel
                    .tag
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || b"_-".contains(&c))
                || !tags.insert(channel.tag.as_str())
            {
                return Err(NotificationError::InvalidConfiguration);
            }
            if channel.enabled {
                if channel
                    .mp_config
                    .get("WECHAT_MODE")
                    .and_then(serde_json::Value::as_str)
                    == Some("bot")
                {
                    super::mp_config::validate_bot(channel)?;
                } else {
                    channel.provider.validate()?;
                }
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ChannelConfig {
    pub tag: String,
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message_format: Option<String>,
    #[serde(default)]
    pub mp_config: BTreeMap<String, serde_json::Value>,
    #[serde(
        default,
        deserialize_with = "super::event_types::deserialize_selection"
    )]
    pub switchs: Vec<String>,
    #[serde(flatten)]
    pub provider: ProviderConfig,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "provider", rename_all = "snake_case")]
pub enum ProviderConfig {
    Telegram {
        #[serde(default = "telegram_url")]
        api_url: String,
        bot_token_env: String,
        chat_id: String,
        #[serde(default)]
        message_thread_id: Option<i64>,
    },
    Slack {
        #[serde(default = "slack_url")]
        api_url: String,
        bot_token_env: String,
        channel: String,
    },
    Discord {
        #[serde(default = "discord_url")]
        api_url: String,
        bot_token_env: String,
        channel_id: String,
    },
    DingTalk {
        webhook_url_env: String,
        #[serde(default)]
        secret_env: Option<String>,
    },
    SynologyChat {
        webhook_url_env: String,
        #[serde(default)]
        user_ids: Vec<u64>,
    },
    VoceChat {
        api_url: String,
        api_key_env: String,
        target: String,
    },
    Wechat {
        #[serde(default = "wechat_url")]
        api_url: String,
        corp_id: String,
        corp_secret_env: String,
        agent_id: u64,
        #[serde(default = "wechat_users")]
        touser: String,
    },
    QyWechatBot {
        webhook_url_env: String,
        #[serde(default = "wechat_bot_message_type")]
        message_type: String,
    },
    Feishu {
        #[serde(default = "feishu_url")]
        api_url: String,
        app_id: String,
        app_secret_env: String,
        receive_id: String,
        #[serde(default = "feishu_id_type")]
        receive_id_type: String,
    },
    WechatClawBot {
        #[serde(default = "clawbot_url")]
        api_url: String,
        bot_token_env: String,
        account_id: String,
        recipient: String,
        #[serde(default)]
        context_token_env: Option<String>,
    },
    Qq {
        #[serde(default = "qq_url")]
        api_url: String,
        #[serde(default = "qq_token_url")]
        token_url: String,
        app_id: String,
        app_secret_env: String,
        recipient: String,
        #[serde(default = "qq_recipient_type")]
        recipient_type: String,
    },
    WebPush {
        subscriptions_env: String,
        private_key_env: String,
        #[serde(default = "webpush_subject")]
        subject: String,
        #[serde(default = "webpush_ttl")]
        ttl_seconds: u32,
    },
    AppPush {
        api_url: String,
        relay_key_env: String,
        owner: String,
        #[serde(default = "app_route")]
        route: String,
    },
    Web {
        receive_url_env: String,
        #[serde(default)]
        token_env: Option<String>,
    },
    WebAgent {
        receive_url_env: String,
        #[serde(default)]
        token_env: Option<String>,
    },
}

impl ProviderConfig {
    pub fn name(&self) -> &'static str {
        match self {
            Self::Telegram { .. } => "telegram",
            Self::Slack { .. } => "slack",
            Self::Discord { .. } => "discord",
            Self::DingTalk { .. } => "ding_talk",
            Self::SynologyChat { .. } => "synology_chat",
            Self::VoceChat { .. } => "voce_chat",
            Self::Wechat { .. } => "wechat",
            Self::QyWechatBot { .. } => "qy_wechat_bot",
            Self::Feishu { .. } => "feishu",
            Self::WechatClawBot { .. } => "wechat_claw_bot",
            Self::Qq { .. } => "qq",
            Self::WebPush { .. } => "web_push",
            Self::AppPush { .. } => "app_push",
            Self::Web { .. } => "web",
            Self::WebAgent { .. } => "web_agent",
        }
    }

    pub(crate) fn required_env(&self) -> Vec<&str> {
        match self {
            Self::Telegram { bot_token_env, .. }
            | Self::Slack { bot_token_env, .. }
            | Self::Discord { bot_token_env, .. }
            | Self::WechatClawBot { bot_token_env, .. } => vec![bot_token_env],
            Self::DingTalk {
                webhook_url_env, ..
            }
            | Self::SynologyChat {
                webhook_url_env, ..
            }
            | Self::QyWechatBot {
                webhook_url_env, ..
            } => vec![webhook_url_env],
            Self::VoceChat { api_key_env, .. } => vec![api_key_env],
            Self::Wechat {
                corp_secret_env, ..
            } => vec![corp_secret_env],
            Self::Feishu { app_secret_env, .. } | Self::Qq { app_secret_env, .. } => {
                vec![app_secret_env]
            }
            Self::WebPush {
                subscriptions_env,
                private_key_env,
                ..
            } => vec![subscriptions_env, private_key_env],
            Self::AppPush { relay_key_env, .. } => vec![relay_key_env],
            Self::Web {
                receive_url_env, ..
            }
            | Self::WebAgent {
                receive_url_env, ..
            } => vec![receive_url_env],
        }
    }

    pub(crate) fn optional_env(&self) -> Vec<&str> {
        match self {
            Self::DingTalk { secret_env, .. } => secret_env.as_deref().into_iter().collect(),
            Self::WechatClawBot {
                context_token_env, ..
            } => context_token_env.as_deref().into_iter().collect(),
            Self::Web { token_env, .. } | Self::WebAgent { token_env, .. } => {
                token_env.as_deref().into_iter().collect()
            }
            _ => Vec::new(),
        }
    }

    fn validate(&self) -> Result<(), NotificationError> {
        for name in self.required_env().into_iter().chain(self.optional_env()) {
            if name.is_empty()
                || !name.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_')
                || name.as_bytes()[0].is_ascii_digit()
            {
                return Err(NotificationError::InvalidConfiguration);
            }
        }
        match self {
            Self::Telegram {
                api_url, chat_id, ..
            } => {
                validate_url(api_url)?;
                nonempty(&[chat_id])
            }
            Self::Slack {
                api_url, channel, ..
            } => {
                validate_url(api_url)?;
                nonempty(&[channel])
            }
            Self::Discord {
                api_url,
                channel_id,
                ..
            } => {
                validate_url(api_url)?;
                identifier(channel_id)
            }
            Self::VoceChat {
                api_url, target, ..
            } => {
                validate_url(api_url)?;
                let id = target
                    .strip_prefix("GID#")
                    .or_else(|| target.strip_prefix("UID#"))
                    .unwrap_or(target);
                identifier(id)
            }
            Self::Wechat {
                api_url,
                corp_id,
                touser,
                agent_id,
                ..
            } => {
                validate_url(api_url)?;
                if *agent_id == 0 {
                    return Err(NotificationError::InvalidConfiguration);
                }
                nonempty(&[corp_id, touser])
            }
            Self::QyWechatBot { message_type, .. } => {
                if !["text", "markdown"].contains(&message_type.as_str()) {
                    return Err(NotificationError::InvalidConfiguration);
                }
                Ok(())
            }
            Self::Feishu {
                api_url,
                app_id,
                receive_id,
                receive_id_type,
                ..
            } => {
                validate_url(api_url)?;
                if !["open_id", "user_id", "union_id", "chat_id", "email"]
                    .contains(&receive_id_type.as_str())
                {
                    return Err(NotificationError::InvalidConfiguration);
                }
                nonempty(&[app_id, receive_id])
            }
            Self::WechatClawBot {
                api_url,
                account_id,
                recipient,
                ..
            } => {
                validate_url(api_url)?;
                nonempty(&[account_id, recipient])
            }
            Self::Qq {
                api_url,
                token_url,
                app_id,
                recipient,
                recipient_type,
                ..
            } => {
                validate_url(api_url)?;
                validate_url(token_url)?;
                if !["user", "group"].contains(&recipient_type.as_str()) {
                    return Err(NotificationError::InvalidConfiguration);
                }
                nonempty(&[app_id, recipient])
            }
            Self::WebPush {
                subject,
                ttl_seconds,
                ..
            } => {
                if !subject.starts_with("mailto:") && !subject.starts_with("https://")
                    || *ttl_seconds > 86400
                {
                    return Err(NotificationError::InvalidConfiguration);
                }
                Ok(())
            }
            Self::AppPush {
                api_url,
                owner,
                route,
                ..
            } => {
                validate_url(api_url)?;
                if !route.starts_with('/') || route.starts_with("//") || route.contains('\\') {
                    return Err(NotificationError::InvalidConfiguration);
                }
                nonempty(&[owner])
            }
            _ => Ok(()),
        }
    }
}

fn nonempty(values: &[&String]) -> Result<(), NotificationError> {
    if values.iter().any(|s| s.trim().is_empty()) {
        Err(NotificationError::InvalidConfiguration)
    } else {
        Ok(())
    }
}
fn identifier(value: &str) -> Result<(), NotificationError> {
    if value.is_empty() || !value.bytes().all(|c| c.is_ascii_digit()) {
        Err(NotificationError::InvalidConfiguration)
    } else {
        Ok(())
    }
}
pub(crate) fn validate_url(value: &str) -> Result<(), NotificationError> {
    let url = reqwest::Url::parse(value).map_err(|_| NotificationError::InvalidConfiguration)?;
    if !["http", "https"].contains(&url.scheme())
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
    {
        return Err(NotificationError::InvalidConfiguration);
    }
    Ok(())
}
fn telegram_url() -> String {
    "https://api.telegram.org".into()
}
fn slack_url() -> String {
    "https://slack.com/api".into()
}
fn discord_url() -> String {
    "https://discord.com/api/v10".into()
}
fn wechat_url() -> String {
    "https://qyapi.weixin.qq.com".into()
}
fn wechat_users() -> String {
    "@all".into()
}
fn wechat_bot_message_type() -> String {
    "text".into()
}
fn feishu_url() -> String {
    "https://open.feishu.cn".into()
}
fn feishu_id_type() -> String {
    "chat_id".into()
}
fn clawbot_url() -> String {
    "https://ilinkai.weixin.qq.com".into()
}
fn qq_url() -> String {
    "https://api.sgroup.qq.com".into()
}
fn qq_token_url() -> String {
    "https://bots.qq.com/app/getAppAccessToken".into()
}
fn qq_recipient_type() -> String {
    "user".into()
}
fn webpush_ttl() -> u32 {
    86400
}
fn webpush_subject() -> String {
    "mailto:admin@example.invalid".into()
}
fn app_route() -> String {
    "/?shotcut=message".into()
}
