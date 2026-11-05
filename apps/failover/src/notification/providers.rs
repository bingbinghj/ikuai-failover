use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::{
    Engine,
    engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD},
};
use hmac::{Hmac, Mac};
use reqwest::Client;
use serde::Deserialize;
use serde_json::json;
use sha2::Sha256;

use super::config::validate_url;
use super::transport::{PreparedChannel, accept, endpoint, json_response, response};
use super::{Event, NotificationError, ProviderConfig};

pub(crate) async fn send(
    client: &Client,
    channel: &PreparedChannel,
    event: &Event,
) -> Result<(), NotificationError> {
    if channel
        .mp_config
        .get("WECHAT_MODE")
        .and_then(serde_json::Value::as_str)
        == Some("bot")
    {
        return super::mp_config::send_bot(channel, event).await;
    }
    let markdown = channel.message_format == "markdown";
    let text = super::formatting::bounded(event, 30_000)?;
    match &channel.provider {
        ProviderConfig::Telegram {
            api_url,
            bot_token_env,
            chat_id,
            message_thread_id,
        } => {
            let token = channel.secret(bot_token_env)?;
            let html = channel.message_format == "html";
            let content = if markdown {
                super::formatting::bounded_rendered(
                    event,
                    4000,
                    super::formatting::telegram_markdown_v2,
                )?
            } else if html {
                super::formatting::bounded_rendered(
                    event,
                    4000,
                    super::formatting::telegram_native_html,
                )?
            } else {
                super::formatting::bounded(event, 4000)?
            };
            let mut body = json!({"chat_id":chat_id,"text":content,"link_preview_options":{"is_disabled":true}});
            if markdown {
                body["parse_mode"] = json!("MarkdownV2");
            } else if html {
                body["parse_mode"] = json!("HTML");
            }
            if let Some(thread) = message_thread_id {
                body["message_thread_id"] = json!(thread);
            }
            let result = json_response(
                client
                    .post(endpoint(api_url, &format!("bot{token}/sendMessage")))
                    .json(&body),
            )
            .await?;
            accept(result["ok"] == true)
        }
        ProviderConfig::Slack {
            api_url,
            bot_token_env,
            channel: target,
        } => {
            let content = super::formatting::bounded_rendered(event, 30_000, |source| {
                if markdown {
                    super::formatting::slack_markdown(source)
                } else {
                    source
                        .replace('&', "&amp;")
                        .replace('<', "&lt;")
                        .replace('>', "&gt;")
                }
            })?;
            let mut body = json!({"channel":target,"text":content,"mrkdwn":markdown,"unfurl_links":false,"unfurl_media":false});
            if markdown && content.chars().count() <= 3000 {
                body["blocks"] =
                    json!([{"type":"section","text":{"type":"mrkdwn","text":content}}]);
            }
            let result = json_response(
                client
                    .post(endpoint(api_url, "chat.postMessage"))
                    .bearer_auth(channel.secret(bot_token_env)?)
                    .json(&body),
            )
            .await?;
            accept(result["ok"] == true)
        }
        ProviderConfig::Discord {
            api_url,
            bot_token_env,
            channel_id,
        } => {
            let body = if channel.message_format == "embed" {
                let title = super::formatting::plain(&event.title);
                let title: String = title.chars().take(256).collect();
                let content = super::formatting::bounded(event, 4000)?;
                let description = content
                    .strip_prefix(&format!("{}\n", event.title))
                    .unwrap_or(&content);
                json!({"embeds":[{"title":title,"description":description}],"allowed_mentions":{"parse":[]}})
            } else {
                json!({"content":if markdown { super::formatting::bounded(event, 1800)? } else { super::formatting::bounded_rendered(event, 1800, super::formatting::escape_markdown)? },"allowed_mentions":{"parse":[]}})
            };
            let result = json_response(
                client
                    .post(endpoint(
                        api_url,
                        &format!("channels/{channel_id}/messages"),
                    ))
                    .header(
                        "Authorization",
                        format!("Bot {}", channel.secret(bot_token_env)?),
                    )
                    .json(&body),
            )
            .await?;
            accept(result["id"].as_str().is_some() && result.get("code").is_none())
        }
        ProviderConfig::DingTalk {
            webhook_url_env,
            secret_env,
        } => {
            let mut url = reqwest::Url::parse(channel.secret(webhook_url_env)?)
                .map_err(|_| NotificationError::InvalidConfiguration)?;
            validate_url(url.as_str())?;
            if let Some(secret) = channel.optional_secret(secret_env)? {
                let timestamp = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map_err(|_| NotificationError::Crypto)?
                    .as_millis()
                    .to_string();
                let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes())
                    .map_err(|_| NotificationError::Crypto)?;
                mac.update(format!("{timestamp}\n{secret}").as_bytes());
                let signature = STANDARD.encode(mac.finalize().into_bytes());
                url.query_pairs_mut()
                    .append_pair("timestamp", &timestamp)
                    .append_pair("sign", &signature);
            }
            let result = json_response(
                client
                    .post(url)
                    .json(&if markdown { json!({"msgtype":"markdown","markdown":{"title":event.title.chars().take(256).collect::<String>(),"text":super::formatting::bounded(event, 12000)?}}) } else { json!({"msgtype":"text","text":{"content":super::formatting::bounded(event, 12000)?}}) }),
            )
            .await?;
            accept(result["errcode"] == 0)
        }
        ProviderConfig::SynologyChat {
            webhook_url_env,
            user_ids,
        } => {
            let url = channel.secret(webhook_url_env)?;
            validate_url(url)?;
            let content = super::formatting::bounded_rendered(event, 30_000, |source| {
                if markdown {
                    super::formatting::synology_markdown(source)
                } else {
                    super::formatting::escape_markdown(source)
                }
            })?;
            let mut body = json!({"text":content});
            if !user_ids.is_empty() {
                body["user_ids"] = json!(user_ids);
            }
            let result =
                json_response(client.post(url).form(&[("payload", body.to_string())])).await?;
            accept(
                result["success"] == true
                    && result.pointer("/error/code").is_none_or(|code| code == 0),
            )
        }
        ProviderConfig::VoceChat {
            api_url,
            api_key_env,
            target,
        } => {
            let (action, id) = match target.strip_prefix("GID#") {
                Some(id) => ("send_to_group", id),
                None => (
                    "send_to_user",
                    target.strip_prefix("UID#").unwrap_or(target),
                ),
            };
            response(
                client
                    .post(endpoint(api_url, &format!("api/bot/{action}/{id}")))
                    .header("x-api-key", channel.secret(api_key_env)?)
                    .header(
                        "Content-Type",
                        if markdown {
                            "text/markdown"
                        } else {
                            "text/plain"
                        },
                    )
                    .body(text),
            )
            .await?;
            Ok(())
        }
        ProviderConfig::QyWechatBot {
            webhook_url_env,
            message_type: _,
        } => {
            let url = channel.secret(webhook_url_env)?;
            validate_url(url)?;
            let maximum_bytes = match channel.message_format.as_str() {
                "text" => 2048,
                "markdown" => 4096,
                _ => return Err(NotificationError::InvalidConfiguration),
            };
            let content = super::formatting::bounded(event, maximum_bytes)?;
            let body = if !markdown {
                json!({"msgtype":"text","text":{"content":content}})
            } else {
                json!({"msgtype":"markdown","markdown":{"content":content}})
            };
            let result = json_response(client.post(url).json(&body)).await?;
            let code = result
                .get("errcode")
                .and_then(|value| value.as_i64())
                .ok_or(NotificationError::InvalidResponse)?;
            accept(code == 0)
        }
        ProviderConfig::Wechat { .. } => super::wechat::send(client, channel, event).await,
        ProviderConfig::Feishu {
            api_url,
            app_id,
            app_secret_env,
            receive_id,
            receive_id_type,
        } => {
            let result = json_response(
                client
                    .post(endpoint(
                        api_url,
                        "open-apis/auth/v3/tenant_access_token/internal",
                    ))
                    .json(&json!({"app_id":app_id,"app_secret":channel.secret(app_secret_env)?})),
            )
            .await?;
            accept(result["code"] == 0)?;
            let token = result["tenant_access_token"]
                .as_str()
                .ok_or(NotificationError::InvalidResponse)?;
            let targets = mp_targets(
                channel,
                &[("FEISHU_OPEN_ID", "open_id"), ("FEISHU_CHAT_ID", "chat_id")],
                receive_id,
                receive_id_type,
            );
            if targets.is_empty() {
                return Err(NotificationError::InvalidConfiguration);
            }
            for (target, target_type) in targets {
                let content = super::formatting::bounded(event, 12_000)?;
                let result = json_response(client.post(endpoint(api_url, "open-apis/im/v1/messages")).query(&[("receive_id_type", target_type)]).bearer_auth(token).json(&if markdown { json!({"receive_id":target,"msg_type":"interactive","content":json!({"config":{"wide_screen_mode":true},"elements":[{"tag":"markdown","content":content}]}).to_string()}) } else { json!({"receive_id":target,"msg_type":"text","content":json!({"text":content}).to_string()}) })).await?;
                accept(result["code"] == 0)?;
            }
            Ok(())
        }
        ProviderConfig::WechatClawBot {
            api_url,
            bot_token_env,
            account_id,
            recipient,
            context_token_env,
        } => {
            let mut msg = json!({"from_user_id":account_id,"to_user_id":recipient,"client_id":event.id,"message_type":2,"message_state":2,"item_list":[{"type":1,"text_item":{"text":super::formatting::bounded_plain(event, 30_000)?}}]});
            if let Some(context) = channel.optional_secret(context_token_env)? {
                msg["context_token"] = json!(context);
            }
            use web_push_native::p256::elliptic_curve::rand_core::{OsRng, RngCore};
            let uin = OsRng.next_u32();
            let result = json_response(
                client
                    .post(endpoint(api_url, "ilink/bot/sendmessage"))
                    .bearer_auth(channel.secret(bot_token_env)?)
                    .header("AuthorizationType", "ilink_bot_token")
                    .header("X-WECHAT-UIN", STANDARD.encode(uin.to_string()))
                    .json(&json!({"msg":msg,"base_info":{"channel_version":"1.0.0"}})),
            )
            .await?;
            accept(
                ["ret", "errcode", "code"]
                    .iter()
                    .all(|key| result.get(*key).is_none_or(|code| code == 0))
                    && (result["ret"] == 0
                        || result["errcode"] == 0
                        || result["code"] == 0
                        || result["success"] == true)
                    && result["success"] != false,
            )
        }
        ProviderConfig::Qq {
            api_url,
            token_url,
            app_id,
            app_secret_env,
            recipient,
            recipient_type,
        } => {
            let result = json_response(
                client
                    .post(token_url)
                    .json(&json!({"appId":app_id,"clientSecret":channel.secret(app_secret_env)?})),
            )
            .await?;
            let token = result["access_token"]
                .as_str()
                .ok_or(NotificationError::InvalidResponse)?;
            let targets = mp_targets(
                channel,
                &[("QQ_OPENID", "user"), ("QQ_GROUP_OPENID", "group")],
                recipient,
                recipient_type,
            );
            if targets.is_empty() {
                return Err(NotificationError::InvalidConfiguration);
            }
            for (target, target_type) in targets {
                let kind = if target_type == "group" {
                    "groups"
                } else {
                    "users"
                };
                let mut url = reqwest::Url::parse(api_url)
                    .map_err(|_| NotificationError::InvalidConfiguration)?;
                url.path_segments_mut()
                    .map_err(|_| NotificationError::InvalidConfiguration)?
                    .pop_if_empty()
                    .extend(["v2", kind, target, "messages"]);
                let native_markdown = markdown
                    && channel
                        .mp_config
                        .get("QQ_MARKDOWN")
                        .and_then(serde_json::Value::as_bool)
                        != Some(false);
                let body = if native_markdown {
                    json!({"msg_type":2,"markdown":{"content":super::formatting::bounded(event, 4000)?}})
                } else {
                    json!({"content":if markdown { super::formatting::bounded_plain(event, 4000)? } else { super::formatting::bounded(event, 4000)? },"msg_type":0})
                };
                let (status, mut result) = super::transport::json_response_with_status(
                    client
                        .post(url.clone())
                        .header("Authorization", format!("QQBot {token}"))
                        .json(&body),
                )
                .await?;
                if native_markdown && (result["code"] == 11244 || result["code"] == "11244") {
                    crate::logging::warn("[通知] QQ 未获原生 Markdown 权限，改为兼容文本。");
                    result = json_response(client.post(url).header("Authorization", format!("QQBot {token}")).json(&json!({"msg_type":0,"content":super::formatting::bounded_plain(event, 4000)?}))).await?;
                } else if !status.is_success() {
                    return Err(NotificationError::HttpRejected);
                }
                accept(result["id"].as_str().is_some() && result.get("code").is_none())?;
            }
            Ok(())
        }
        ProviderConfig::WebPush {
            subscriptions_env,
            private_key_env,
            subject,
            ttl_seconds,
        } => {
            webpush(
                client,
                channel,
                subscriptions_env,
                private_key_env,
                subject,
                *ttl_seconds,
                event,
            )
            .await
        }
        ProviderConfig::AppPush {
            api_url,
            relay_key_env,
            owner,
            route,
        } => {
            let key = channel.secret(relay_key_env)?;
            if key.len() < 32 {
                return Err(NotificationError::InvalidConfiguration);
            }
            let title = super::formatting::plain(&event.title);
            let message = super::formatting::bounded_plain(event, 30_000)?;
            let body = message
                .strip_prefix(&format!("{title}\n"))
                .unwrap_or(&message);
            let result = json_response(
                client
                    .post(endpoint(api_url, "push/messages"))
                    .bearer_auth(key)
                    .json(&json!({"owner":owner,"title":title,"body":body,"route":route})),
            )
            .await?;
            accept(
                result["accepted"].as_u64().is_some_and(|n| n > 0)
                    && result["failed"].as_array().is_none_or(Vec::is_empty)
                    && result["success"] != false,
            )
        }
        ProviderConfig::Web {
            receive_url_env,
            token_env,
        }
        | ProviderConfig::WebAgent {
            receive_url_env,
            token_env,
        } => {
            let url = channel.secret(receive_url_env)?;
            validate_url(url)?;
            let mut event = event.clone();
            let format = if channel.provider.name() == "web" {
                event.title = super::formatting::plain(&event.title);
                event.text = super::formatting::plain(&event.text);
                "text"
            } else {
                channel.message_format.as_str()
            };
            let request = client.post(url).json(
                &json!({"channel":channel.provider.name(),"message_format":format,"event":event}),
            );
            let request = if let Some(token) = channel.optional_secret(token_env)? {
                request.bearer_auth(token)
            } else {
                request
            };
            let result = json_response(request).await?;
            accept(
                (result["success"] == true || result["ok"] == true)
                    && result["success"] != false
                    && result["ok"] != false,
            )
        }
    }
}

#[derive(Deserialize)]
struct Subscription {
    endpoint: String,
    keys: SubscriptionKeys,
}
#[derive(Deserialize)]
struct SubscriptionKeys {
    p256dh: String,
    auth: String,
}

async fn webpush(
    client: &Client,
    channel: &PreparedChannel,
    subscriptions_env: &str,
    private_key_env: &str,
    subject: &str,
    ttl_seconds: u32,
    event: &Event,
) -> Result<(), NotificationError> {
    use web_push_native::{
        Auth, WebPushBuilder, jwt_simple::algorithms::ES256KeyPair, p256::PublicKey,
    };
    let subscriptions: Vec<Subscription> = serde_json::from_str(channel.secret(subscriptions_env)?)
        .map_err(|_| NotificationError::InvalidConfiguration)?;
    if subscriptions.is_empty() || subscriptions.len() > 32 {
        return Err(NotificationError::InvalidConfiguration);
    }
    let private = URL_SAFE_NO_PAD
        .decode(channel.secret(private_key_env)?)
        .map_err(|_| NotificationError::Crypto)?;
    let key = ES256KeyPair::from_bytes(&private).map_err(|_| NotificationError::Crypto)?;
    let content = webpush_content(event, channel.message_format == "markdown")?;
    let mut failed = false;
    for subscription in subscriptions {
        validate_url(&subscription.endpoint)?;
        let public = URL_SAFE_NO_PAD
            .decode(&subscription.keys.p256dh)
            .map_err(|_| NotificationError::Crypto)?;
        let public = PublicKey::from_sec1_bytes(&public).map_err(|_| NotificationError::Crypto)?;
        let auth: [u8; 16] = URL_SAFE_NO_PAD
            .decode(&subscription.keys.auth)
            .map_err(|_| NotificationError::Crypto)?
            .try_into()
            .map_err(|_| NotificationError::Crypto)?;
        let request = WebPushBuilder::new(
            subscription
                .endpoint
                .parse()
                .map_err(|_| NotificationError::InvalidConfiguration)?,
            public,
            Auth::from(auth),
        )
        .with_valid_duration(Duration::from_secs(u64::from(ttl_seconds)))
        .with_vapid(&key, subject)
        .build(content.clone())
        .map_err(|_| NotificationError::Crypto)?;
        let (parts, body) = request.into_parts();
        let mut request = client
            .post(parts.uri.to_string())
            .headers(parts.headers)
            .body(body);
        if reqwest::Url::parse(&subscription.endpoint)
            .ok()
            .and_then(|url| url.host_str().map(str::to_owned))
            .is_some_and(|host| {
                host == "notify.windows.com" || host.ends_with(".notify.windows.com")
            })
        {
            request = request.header("X-WNS-Cache-Policy", "cache");
        }
        if response(request).await.is_err() {
            failed = true;
        }
    }
    accept(!failed)
}

pub(crate) fn webpush_content(event: &Event, _markdown: bool) -> Result<String, NotificationError> {
    let title = super::formatting::plain(&event.title);
    let mut limit = 2800;
    for _ in 0..12 {
        let message = super::formatting::bounded_plain(event, limit)?;
        let body = message
            .strip_prefix(&format!("{title}\n"))
            .unwrap_or(&message);
        // System notification surfaces render text, even when the source template was Markdown.
        let content = json!({"title":title,"body":body,"message_format":"text","url":"/#/logs/notifications"}).to_string();
        if content.len() <= 3500 {
            return Ok(content);
        }
        limit = limit.saturating_mul(3500) / content.len();
        limit = limit.saturating_sub(32);
    }
    Err(NotificationError::InvalidConfiguration)
}

fn mp_targets<'a>(
    channel: &'a PreparedChannel,
    bindings: &[(&str, &'a str)],
    fallback: &'a str,
    fallback_type: &'a str,
) -> Vec<(&'a str, &'a str)> {
    if bindings
        .iter()
        .any(|(key, _)| channel.mp_config.contains_key(*key))
    {
        bindings
            .iter()
            .filter_map(|(key, kind)| {
                channel
                    .mp_config
                    .get(*key)
                    .and_then(serde_json::Value::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(|value| (value, *kind))
            })
            .collect()
    } else if !fallback.trim().is_empty() {
        vec![(fallback, fallback_type)]
    } else {
        Vec::new()
    }
}
