use super::{ChannelConfig, ProviderConfig};

pub(crate) fn default(channel: &ChannelConfig) -> &str {
    if channel.provider.name() == "wechat"
        && channel
            .mp_config
            .get("WECHAT_MODE")
            .and_then(serde_json::Value::as_str)
            == Some("bot")
    {
        return "markdown";
    }
    match &channel.provider {
        ProviderConfig::Discord { .. } => "embed",
        ProviderConfig::Telegram { .. }
        | ProviderConfig::Slack { .. }
        | ProviderConfig::DingTalk { .. }
        | ProviderConfig::SynologyChat { .. }
        | ProviderConfig::VoceChat { .. }
        | ProviderConfig::Feishu { .. }
        | ProviderConfig::Qq { .. }
        | ProviderConfig::WebAgent { .. } => "markdown",
        ProviderConfig::QyWechatBot { message_type, .. } => message_type,
        _ => "text",
    }
}

pub(crate) fn valid(channel: &ChannelConfig, format: &str) -> bool {
    // Text/Markdown remain accepted for existing installations. Text-only channels convert old Markdown.
    match format {
        "text" | "markdown" => true,
        "html" => channel.provider.name() == "telegram",
        "embed" => channel.provider.name() == "discord",
        "news" => {
            channel.provider.name() == "wechat"
                && channel
                    .mp_config
                    .get("WECHAT_MODE")
                    .and_then(serde_json::Value::as_str)
                    != Some("bot")
        }
        _ => false,
    }
}

pub(crate) fn effective(channel: &ChannelConfig) -> String {
    channel
        .message_format
        .as_deref()
        .unwrap_or_else(|| default(channel))
        .to_owned()
}
