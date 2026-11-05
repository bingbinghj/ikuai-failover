//! Browser Web Push enrollment. These routes are protected by the runtime auth middleware.
//! The private VAPID key and subscription credentials never leave the configuration store.
use std::{
    path::Path,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use axum::{
    Json,
    extract::{
        Query, State,
        rejection::{JsonRejection, QueryRejection},
    },
    http::{StatusCode, header},
    response::{IntoResponse, Response},
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use tokio::io::AsyncReadExt;
use web_push_native::{
    jwt_simple::algorithms::ES256KeyPair,
    p256::{PublicKey, elliptic_curve::sec1::ToEncodedPoint},
};

use super::{NotificationConfig, ProviderConfig};
use crate::{config, runtime::Runtime};

const MAX_CONFIG_BYTES: u64 = 2 * 1024 * 1024;
const MAX_SUBSCRIPTIONS: usize = 32;
const DEFAULT_KEY_NAME: &str = "IKF_WEBPUSH_VAPID_KEY";
const DEFAULT_SUBJECT: &str = "mailto:admin@example.invalid";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KeyRequest {
    tag: String,
}

#[derive(Clone, Deserialize, Serialize)]
pub struct BrowserSubscription {
    endpoint: String,
    #[serde(rename = "expirationTime", default)]
    expiration_time: Option<u64>,
    keys: SubscriptionKeys,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct SubscriptionKeys {
    p256dh: String,
    auth: String,
}

#[derive(Clone, Deserialize, Serialize)]
struct StoredSubscription {
    #[serde(flatten)]
    subscription: BrowserSubscription,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    username: Option<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubscribeRequest {
    tag: String,
    username: String,
    subscription: BrowserSubscription,
}

struct SettingsError {
    status: StatusCode,
    code: &'static str,
    message: &'static str,
}
impl SettingsError {
    fn invalid(message: &'static str) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            code: "invalid_webpush_settings",
            message,
        }
    }
    fn read() -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            code: "webpush_configuration_unavailable",
            message: "无法读取通知配置，请检查配置文件。",
        }
    }
    fn changed() -> Self {
        Self {
            status: StatusCode::CONFLICT,
            code: "configuration_changed",
            message: "配置已被修改，请重新加载后再开启浏览器通知。",
        }
    }
    fn response(self) -> Response {
        no_store(
            (
                self.status,
                Json(json!({"success":false,"error":self.code,"message":self.message})),
            )
                .into_response(),
        )
    }
}

/// Read an already initialized channel's public key without changing configuration.
pub async fn public_key(
    State(runtime): State<Arc<Runtime>>,
    query: Result<Query<KeyRequest>, QueryRejection>,
) -> Response {
    let Ok(Query(request)) = query else {
        return SettingsError::invalid("请选择浏览器通知渠道。").response();
    };
    if !valid_tag(&request.tag) {
        return SettingsError::invalid("请选择有效的通知渠道。").response();
    }
    let _gate = runtime.settings_gate.lock().await;
    let result = async {
        let (bytes, document) = read_document(&runtime.config.source_path).await?;
        let notifications = notification_document(&document)?;
        let index = channel_index(&notifications, &request.tag)?;
        let ProviderConfig::WebPush { private_key_env, .. } = &notifications.channels[index].provider else { unreachable!() };
        let Some(private) = secret(&notifications, private_key_env) else {
            return Err(SettingsError { status: StatusCode::NOT_FOUND, code: "webpush_key_not_initialized", message: "请点击开启浏览器通知以初始化此渠道。" });
        };
        let public_key = public_key_from_private(&private)?;
        Ok(json!({"success":true,"tag":request.tag,"public_key":public_key,"revision":revision(&bytes)}))
    }.await;
    response(result)
}

/// A user's explicit enrollment action initializes and persists the VAPID key.
pub async fn initialize_key(
    State(runtime): State<Arc<Runtime>>,
    request: Result<Json<KeyRequest>, JsonRejection>,
) -> Response {
    let Ok(Json(request)) = request else {
        return SettingsError::invalid("请选择浏览器通知渠道。").response();
    };
    if !valid_tag(&request.tag) {
        return SettingsError::invalid("请选择有效的通知渠道。").response();
    }
    let _gate = runtime.settings_gate.lock().await;
    let result = async {
        let path = &runtime.config.source_path;
        let (bytes, _) = read_document(path).await?;
        let tag = request.tag.clone();
        let saved = config::update_document(path, Some(&bytes), move |document| {
            let mut notifications = notification_document(document).map_err(|error| error.code.to_owned())?;
            initialize_channel(&mut notifications, &tag).map_err(|error| error.code.to_owned())?;
            write_notifications(document, notifications).map_err(|error| error.code.to_owned())
        }).await.map_err(write_error)?;
        let notifications = notification_document(&saved)?;
        let index = channel_index(&notifications, &request.tag)?;
        let ProviderConfig::WebPush { private_key_env, .. } = &notifications.channels[index].provider else { unreachable!() };
        let private = secret(&notifications, private_key_env).ok_or_else(SettingsError::read)?;
        let public_key = public_key_from_private(&private)?;
        let (saved_bytes, _) = read_document(path).await?;
        Ok(json!({"success":true,"tag":request.tag,"public_key":public_key,"revision":revision(&saved_bytes)}))
    }.await;
    response(result)
}

/// Merge this browser's subscription into the saved channel, preserving other devices.
pub async fn subscribe(
    State(runtime): State<Arc<Runtime>>,
    request: Result<Json<SubscribeRequest>, JsonRejection>,
) -> Response {
    let Ok(Json(mut request)) = request else {
        return SettingsError::invalid("浏览器订阅信息有误，请重新开启通知。").response();
    };
    request.username = request.username.trim().to_owned();
    if !valid_tag(&request.tag)
        || request.username.is_empty()
        || request.username.len() > 256
        || request.username.chars().any(char::is_control)
    {
        return SettingsError::invalid("请填写接收浏览器通知的用户名。").response();
    }
    if let Err(error) = validate_subscription(&request.subscription) {
        return error.response();
    }
    let _gate = runtime.settings_gate.lock().await;
    let result = async {
        let path = &runtime.config.source_path;
        let (bytes, _) = read_document(path).await?;
        let tag = request.tag.clone();
        let username = request.username.clone();
        config::update_document(path, Some(&bytes), move |document| {
            let mut notifications = notification_document(document).map_err(|error| error.code.to_owned())?;
            initialize_channel(&mut notifications, &tag).map_err(|error| error.code.to_owned())?;
            let index = channel_index(&notifications, &tag).map_err(|error| error.code.to_owned())?;
            let ProviderConfig::WebPush { subscriptions_env, .. } = &notifications.channels[index].provider else { unreachable!() };
            let subscriptions_name = subscriptions_env.clone();
            let previous_username = notifications.channels[index].mp_config.get("WEBPUSH_USERNAME").and_then(serde_json::Value::as_str).unwrap_or("").to_owned();
            let mut subscriptions = stored_subscriptions(secret(&notifications, &subscriptions_name).as_deref()).map_err(|error| error.code.to_owned())?;
            let now = now_ms();
            subscriptions.retain(|entry| entry.subscription.expiration_time.is_none_or(|expires| expires > now)
                && entry.username.as_deref().map_or(previous_username.is_empty() || previous_username == username, |owner| owner == username));
            let incoming = StoredSubscription { subscription: request.subscription, username: Some(username.clone()) };
            if let Some(index) = subscriptions.iter().position(|entry| entry.subscription.endpoint == incoming.subscription.endpoint) {
                subscriptions[index] = incoming;
            } else {
                if subscriptions.len() >= MAX_SUBSCRIPTIONS { return Err("webpush_subscription_limit".into()); }
                subscriptions.push(incoming);
            }
            let encoded = serde_json::to_string(&subscriptions).map_err(|_| "invalid_webpush_settings".to_string())?;
            notifications.secrets.insert(subscriptions_name, encoded);
            notifications.channels[index].mp_config.insert("WEBPUSH_USERNAME".into(), json!(username));
            write_notifications(document, notifications).map_err(|error| error.code.to_owned())
        }).await.map_err(write_error)?;
        let (saved_bytes, _) = read_document(path).await?;
        Ok(json!({"success":true,"tag":request.tag,"username":request.username,"restart_required":true,"revision":revision(&saved_bytes),"message":"浏览器通知已开启，重启服务后使用新订阅。"}))
    }.await;
    response(result)
}

fn initialize_channel(
    notifications: &mut NotificationConfig,
    tag: &str,
) -> Result<(), SettingsError> {
    let index = channel_index(notifications, tag)?;
    let ProviderConfig::WebPush {
        private_key_env,
        subscriptions_env,
        subject,
        ..
    } = &notifications.channels[index].provider
    else {
        unreachable!()
    };
    let old_key_name = private_key_env.clone();
    let old_subscriptions_name = subscriptions_env.clone();
    let configured_subject = subject.clone();
    let (key_name, private) = if let Some(private) = secret(notifications, &old_key_name) {
        (old_key_name, private)
    } else if let Some(private) = secret(notifications, DEFAULT_KEY_NAME) {
        (DEFAULT_KEY_NAME.to_owned(), private)
    } else {
        (
            owned_name(DEFAULT_KEY_NAME)?,
            URL_SAFE_NO_PAD.encode(ES256KeyPair::generate().to_bytes()),
        )
    };
    public_key_from_private(&private)?;
    let existing = secret(notifications, &old_subscriptions_name);
    let subscriptions = stored_subscriptions(existing.as_deref())?;
    let subscriptions_name = if valid_secret_name(&old_subscriptions_name)
        && std::env::var(&old_subscriptions_name)
            .ok()
            .filter(|value| !value.trim().is_empty())
            .is_none()
    {
        old_subscriptions_name
    } else {
        let hash = format!("{:x}", Sha256::digest(tag.as_bytes()));
        owned_name(&format!("IKF_WEBPUSH_SUBS_{}", &hash[..16]))?
    };
    notifications.secrets.insert(key_name.clone(), private);
    notifications.secrets.insert(
        subscriptions_name.clone(),
        serde_json::to_string(&subscriptions)
            .map_err(|_| SettingsError::invalid("浏览器订阅信息有误。"))?,
    );
    let ProviderConfig::WebPush {
        private_key_env,
        subscriptions_env,
        subject,
        ..
    } = &mut notifications.channels[index].provider
    else {
        unreachable!()
    };
    *private_key_env = key_name;
    *subscriptions_env = subscriptions_name;
    if configured_subject.is_empty()
        || !(configured_subject.starts_with("mailto:")
            || configured_subject.starts_with("https://"))
    {
        *subject = DEFAULT_SUBJECT.into();
    }
    Ok(())
}

fn owned_name(base: &str) -> Result<String, SettingsError> {
    for index in 0..32 {
        let name = if index == 0 {
            base.to_owned()
        } else {
            format!("{base}_{index}")
        };
        if std::env::var(&name)
            .ok()
            .filter(|value| !value.trim().is_empty())
            .is_none()
        {
            // Environment overrides stay untouched; the generated names belong to this app.
            return Ok(name);
        }
    }
    Err(SettingsError::invalid(
        "无法分配浏览器通知配置，请检查服务环境变量。",
    ))
}
fn valid_tag(tag: &str) -> bool {
    !tag.is_empty()
        && tag.len() <= 64
        && tag
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"_-".contains(&byte))
}
fn valid_secret_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 128
        && !name.as_bytes()[0].is_ascii_digit()
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}
fn channel_index(notifications: &NotificationConfig, tag: &str) -> Result<usize, SettingsError> {
    let Some(index) = notifications
        .channels
        .iter()
        .position(|channel| channel.tag == tag)
    else {
        return Err(SettingsError {
            status: StatusCode::NOT_FOUND,
            code: "webpush_channel_not_saved",
            message: "请先确定并保存此通知渠道，再开启浏览器通知。",
        });
    };
    if !matches!(
        &notifications.channels[index].provider,
        ProviderConfig::WebPush { .. }
    ) {
        return Err(SettingsError::invalid("此渠道不是浏览器通知，请重新选择。"));
    }
    Ok(index)
}
fn secret(config: &NotificationConfig, name: &str) -> Option<String> {
    if !valid_secret_name(name) {
        return None;
    }
    std::env::var(name)
        .ok()
        .filter(|value| !value.trim().is_empty())
        .or_else(|| {
            config
                .secrets
                .get(name)
                .filter(|value| !value.trim().is_empty())
                .cloned()
        })
}
fn public_key_from_private(private: &str) -> Result<String, SettingsError> {
    let private = URL_SAFE_NO_PAD
        .decode(private)
        .map_err(|_| SettingsError::invalid("浏览器通知密钥无效，请重新添加此渠道。"))?;
    let key = ES256KeyPair::from_bytes(&private)
        .map_err(|_| SettingsError::invalid("浏览器通知密钥无效，请重新添加此渠道。"))?;
    let public = PublicKey::from_sec1_bytes(&key.public_key().to_bytes())
        .map_err(|_| SettingsError::invalid("浏览器通知密钥无效。"))?;
    Ok(URL_SAFE_NO_PAD.encode(public.to_encoded_point(false).as_bytes()))
}
fn validate_subscription(subscription: &BrowserSubscription) -> Result<(), SettingsError> {
    let endpoint = reqwest::Url::parse(&subscription.endpoint)
        .map_err(|_| SettingsError::invalid("浏览器推送地址无效。"))?;
    if subscription.endpoint.len() > 4096
        || endpoint.scheme() != "https"
        || endpoint.host_str().is_none()
        || !endpoint.username().is_empty()
        || endpoint.password().is_some()
        || endpoint.fragment().is_some()
    {
        return Err(SettingsError::invalid("浏览器推送地址无效。"));
    }
    if subscription.keys.p256dh.len() > 128 || subscription.keys.auth.len() > 64 {
        return Err(SettingsError::invalid("浏览器订阅密钥无效。"));
    }
    let public = URL_SAFE_NO_PAD
        .decode(&subscription.keys.p256dh)
        .map_err(|_| SettingsError::invalid("浏览器订阅密钥无效。"))?;
    let auth = URL_SAFE_NO_PAD
        .decode(&subscription.keys.auth)
        .map_err(|_| SettingsError::invalid("浏览器订阅密钥无效。"))?;
    if public.len() != 65
        || public[0] != 4
        || PublicKey::from_sec1_bytes(&public).is_err()
        || auth.len() != 16
    {
        return Err(SettingsError::invalid("浏览器订阅密钥无效。"));
    }
    if subscription
        .expiration_time
        .is_some_and(|expires| expires <= now_ms())
    {
        return Err(SettingsError::invalid("浏览器订阅已过期，请重新开启通知。"));
    }
    Ok(())
}
fn stored_subscriptions(value: Option<&str>) -> Result<Vec<StoredSubscription>, SettingsError> {
    let mut subscriptions: Vec<StoredSubscription> = match value {
        None => Vec::new(),
        Some(value) => serde_json::from_str(value)
            .map_err(|_| SettingsError::invalid("已有浏览器订阅无效，请重新添加此渠道。"))?,
    };
    let now = now_ms();
    subscriptions.retain(|entry| {
        entry
            .subscription
            .expiration_time
            .is_none_or(|expires| expires > now)
    });
    if subscriptions.len() > MAX_SUBSCRIPTIONS {
        return Err(SettingsError::invalid("浏览器订阅数量超过限制。"));
    }
    for subscription in &subscriptions {
        validate_subscription(&subscription.subscription)?;
    }
    Ok(subscriptions)
}
fn notification_document(
    document: &serde_yaml::Value,
) -> Result<NotificationConfig, SettingsError> {
    let value = document
        .as_mapping()
        .and_then(|root| root.get(&serde_yaml::Value::String("notifications".into())))
        .cloned()
        .unwrap_or_else(|| serde_yaml::Value::Mapping(Default::default()));
    serde_yaml::from_value(value).map_err(|_| SettingsError::read())
}
fn write_notifications(
    document: &mut serde_yaml::Value,
    notifications: NotificationConfig,
) -> Result<(), SettingsError> {
    notifications
        .validate()
        .map_err(|_| SettingsError::invalid("通知配置有误，请检查已保存的渠道。"))?;
    let root = document.as_mapping_mut().ok_or_else(SettingsError::read)?;
    root.insert(
        serde_yaml::Value::String("notifications".into()),
        serde_yaml::to_value(notifications).map_err(|_| SettingsError::read())?,
    );
    config::validate_candidate(document)
        .map_err(|_| SettingsError::invalid("配置文件有误，请检查后重新开启通知。"))?;
    let encoded = serde_yaml::to_string(document).map_err(|_| SettingsError::read())?;
    if encoded.len() as u64 > MAX_CONFIG_BYTES {
        return Err(SettingsError::invalid("配置文件已超过大小限制。"));
    }
    Ok(())
}
async fn read_document(path: &Path) -> Result<(Vec<u8>, serde_yaml::Value), SettingsError> {
    if path.as_os_str().is_empty() {
        return Err(SettingsError::read());
    }
    let file = tokio::fs::File::open(path)
        .await
        .map_err(|_| SettingsError::read())?;
    let mut bytes = Vec::new();
    file.take(MAX_CONFIG_BYTES + 1)
        .read_to_end(&mut bytes)
        .await
        .map_err(|_| SettingsError::read())?;
    if bytes.len() as u64 > MAX_CONFIG_BYTES {
        return Err(SettingsError::read());
    }
    let document = serde_yaml::from_slice(&bytes).map_err(|_| SettingsError::read())?;
    Ok((bytes, document))
}
fn write_error(error: String) -> SettingsError {
    if error == config::CONFIG_FILE_CHANGED {
        return SettingsError::changed();
    }
    match error.as_str() {
        "webpush_channel_not_saved" => SettingsError {
            status: StatusCode::NOT_FOUND,
            code: "webpush_channel_not_saved",
            message: "请先确定并保存此通知渠道，再开启浏览器通知。",
        },
        "webpush_configuration_unavailable" => SettingsError::read(),
        "webpush_subscription_limit" => {
            SettingsError::invalid("此渠道已达到 32 个浏览器订阅，请移除不用的订阅后再试。")
        }
        "invalid_webpush_settings" => {
            SettingsError::invalid("浏览器推送配置无效，请检查已保存的渠道。")
        }
        _ => SettingsError {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            code: "webpush_save_failed",
            message: "无法保存浏览器通知，请检查配置目录是否可写。",
        },
    }
}
fn revision(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(u128::from(u64::MAX)) as u64)
        .unwrap_or_default()
}
fn response(result: Result<serde_json::Value, SettingsError>) -> Response {
    match result {
        Ok(value) => no_store(Json(value).into_response()),
        Err(error) => error.response(),
    }
}
fn no_store(mut response: Response) -> Response {
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        axum::http::HeaderValue::from_static("no-store"),
    );
    response
}
