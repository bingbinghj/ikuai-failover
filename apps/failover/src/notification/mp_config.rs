use super::{ChannelConfig, Event, NotificationError, transport::PreparedChannel};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};

fn text<'a>(options: &'a std::collections::BTreeMap<String, Value>, key: &str) -> &'a str {
    options.get(key).and_then(Value::as_str).unwrap_or_default()
}

pub fn validate_bot(channel: &ChannelConfig) -> Result<(), NotificationError> {
    if channel.provider.name() != "wechat"
        || ["WECHAT_BOT_ID", "WECHAT_BOT_SECRET", "WECHAT_BOT_CHAT_ID"]
            .iter()
            .any(|key| text(&channel.mp_config, key).trim().is_empty())
    {
        return Err(NotificationError::InvalidConfiguration);
    }
    let url = text(&channel.mp_config, "WECHAT_BOT_WS_URL");
    let url = if url.is_empty() {
        "wss://openws.work.weixin.qq.com"
    } else {
        url
    };
    if !reqwest::Url::parse(url).is_ok_and(|url| {
        url.scheme() == "wss"
            && url.host_str().is_some()
            && url.username().is_empty()
            && url.password().is_none()
    }) {
        return Err(NotificationError::InvalidConfiguration);
    }
    Ok(())
}

pub async fn send_bot(channel: &PreparedChannel, event: &Event) -> Result<(), NotificationError> {
    let options = &channel.mp_config;
    let url = text(options, "WECHAT_BOT_WS_URL");
    let url = if url.is_empty() {
        "wss://openws.work.weixin.qq.com"
    } else {
        url
    };
    let (mut socket, _) = tokio_tungstenite::connect_async(url)
        .await
        .map_err(|_| NotificationError::Network)?;
    let subscribe_id = format!("aibot_subscribe_{}", event.id);
    let send_id = format!("aibot_send_msg_{}", event.id);
    let target = text(options, "WECHAT_BOT_CHAT_ID");
    let (chat_type, chat_id) = if let Some(target) = target.strip_prefix("group:") {
        (2, target)
    } else {
        (1, target.strip_prefix("user:").unwrap_or(target))
    };
    // The smart-bot endpoint renders Markdown; escape Text to preserve its literal content.
    let body = if channel.message_format == "markdown" {
        super::formatting::bounded(event, 3500)?
    } else {
        super::formatting::bounded_rendered(event, 3500, super::formatting::escape_markdown)?
    };
    for (id, body) in [
        (
            subscribe_id,
            json!({"bot_id":text(options,"WECHAT_BOT_ID"),"secret":text(options,"WECHAT_BOT_SECRET")}),
        ),
        (
            send_id,
            json!({"chatid":chat_id,"chat_type":chat_type,"msgtype":"markdown","markdown":{"content":body}}),
        ),
    ] {
        let command = if id.starts_with("aibot_subscribe") {
            "aibot_subscribe"
        } else {
            "aibot_send_msg"
        };
        socket
            .send(tokio_tungstenite::tungstenite::Message::Text(
                json!({"cmd":command,"headers":{"req_id":id},"body":body})
                    .to_string()
                    .into(),
            ))
            .await
            .map_err(|_| NotificationError::Network)?;
        loop {
            let message = socket
                .next()
                .await
                .ok_or(NotificationError::InvalidResponse)?
                .map_err(|_| NotificationError::Network)?;
            if message.is_ping() {
                socket
                    .send(tokio_tungstenite::tungstenite::Message::Pong(
                        message.into_data(),
                    ))
                    .await
                    .map_err(|_| NotificationError::Network)?;
                continue;
            }
            if !message.is_text() {
                continue;
            }
            let reply: Value = serde_json::from_slice(&message.into_data())
                .map_err(|_| NotificationError::InvalidResponse)?;
            if reply.pointer("/headers/req_id").and_then(Value::as_str) != Some(id.as_str()) {
                continue;
            }
            if reply.get("errcode").and_then(Value::as_i64) != Some(0) {
                return Err(NotificationError::ProviderRejected);
            }
            break;
        }
    }
    let _ = socket.close(None).await;
    Ok(())
}
