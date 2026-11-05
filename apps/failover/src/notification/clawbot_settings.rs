use std::{
    collections::BTreeMap,
    sync::{Arc, OnceLock},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use axum::{
    Json,
    extract::{Query, State, rejection::JsonRejection},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::Mutex;

use super::{ChannelConfig, ProviderConfig};
use crate::{config, runtime::Runtime};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TagRequest {
    pub tag: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PollRequest {
    pub tag: String,
    pub qrcode: String,
}

#[derive(Clone)]
struct QrState {
    qrcode: String,
    url: String,
    status: String,
    updated_at: u64,
    base_url: String,
}

struct Settings {
    channel: ChannelConfig,
    token_key: String,
    base_url: String,
    account_id: String,
    connected: bool,
    timeout: Duration,
}

static QR_STATES: OnceLock<Mutex<BTreeMap<String, QrState>>> = OnceLock::new();
fn states() -> &'static Mutex<BTreeMap<String, QrState>> {
    QR_STATES.get_or_init(|| Mutex::new(BTreeMap::new()))
}
fn key(runtime: &Runtime, tag: &str) -> String {
    format!("{}\0{tag}", runtime.config.source_path.display())
}
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs())
}
fn failure(status: StatusCode, message: &str) -> Response {
    output(status, json!({"success":false,"message":message}))
}
fn output(status: StatusCode, value: Value) -> Response {
    (status, [(header::CACHE_CONTROL, "no-store")], Json(value)).into_response()
}
fn valid_tag(tag: &str) -> bool {
    !tag.is_empty()
        && tag.len() <= 64
        && tag
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"_-".contains(&c))
}

async fn document(runtime: &Runtime) -> Result<(Vec<u8>, serde_yaml::Value), String> {
    if runtime.config.source_path.as_os_str().is_empty() {
        return Err("请先保存通知渠道配置。".into());
    }
    let bytes = tokio::fs::read(&runtime.config.source_path)
        .await
        .map_err(|_| "无法读取通知配置。")?;
    if bytes.len() > 2 * 1024 * 1024 {
        return Err("配置文件过大。".into());
    }
    let value = serde_yaml::from_slice(&bytes).map_err(|_| "通知配置格式无效。")?;
    Ok((bytes, value))
}

fn read_settings(document: &serde_yaml::Value, tag: &str) -> Result<Settings, String> {
    if !valid_tag(tag) {
        return Err("通知渠道标识无效。".into());
    }
    let channel = document["notifications"]["channels"]
        .as_sequence()
        .and_then(|channels| {
            channels
                .iter()
                .find(|channel| channel["tag"].as_str() == Some(tag))
        })
        .ok_or("请先确定并保存该通知渠道，再扫码登录。")?;
    let channel: ChannelConfig =
        serde_yaml::from_value(channel.clone()).map_err(|_| "该通知渠道配置不完整。")?;
    let ProviderConfig::WechatClawBot {
        api_url,
        bot_token_env,
        account_id,
        ..
    } = &channel.provider
    else {
        return Err("该渠道不是微信 ClawBot。".into());
    };
    let base_url = channel
        .mp_config
        .get("WECHATCLAWBOT_BASE_URL")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .unwrap_or(api_url)
        .trim_end_matches('/')
        .to_string();
    validate_url(&base_url)?;
    let token = document["notifications"]["secrets"][bot_token_env.as_str()]
        .as_str()
        .unwrap_or("");
    let connected = std::env::var(bot_token_env)
        .ok()
        .is_some_and(|value| !value.trim().is_empty())
        || !token.is_empty();
    let seconds = channel
        .mp_config
        .get("WECHATCLAWBOT_POLL_TIMEOUT")
        .and_then(|value| {
            value
                .as_u64()
                .or_else(|| value.as_str().and_then(|text| text.parse().ok()))
        })
        .unwrap_or(25)
        .clamp(1, 60);
    Ok(Settings {
        token_key: bot_token_env.clone(),
        account_id: account_id.clone(),
        connected,
        base_url,
        timeout: Duration::from_secs(seconds),
        channel,
    })
}

fn validate_url(value: &str) -> Result<(), String> {
    let url = reqwest::Url::parse(value).map_err(|_| "iLink 地址无效。")?;
    if !matches!(url.scheme(), "https" | "http")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err("iLink 地址需要是有效的 HTTP 或 HTTPS 地址。".into());
    }
    Ok(())
}

async fn remote(settings: &Settings, path: &str, query: &[(&str, &str)]) -> Result<Value, String> {
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(10))
        .timeout(settings.timeout)
        .user_agent("MoviePilot-WechatClawBot/1.0")
        .build()
        .map_err(|_| "无法初始化 iLink 连接。")?;
    let mut response = client
        .get(format!("{}{path}", settings.base_url))
        .header("Accept", "application/json")
        .query(query)
        .send()
        .await
        .map_err(|_| "iLink 请求失败或超时，请稍后重试。")?;
    if !response.status().is_success() {
        return Err("iLink 服务拒绝了该请求。".into());
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| "无法读取 iLink 响应。")?
    {
        if bytes.len().saturating_add(chunk.len()) > 262_144 {
            return Err("iLink 响应过大。".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&bytes).map_err(|_| "iLink 返回了无效响应。".into())
}

fn data(value: &Value) -> &Value {
    value
        .get("data")
        .or_else(|| value.get("result"))
        .filter(|value| value.is_object())
        .unwrap_or(value)
}
fn text(value: &Value, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| {
        value
            .get(*key)
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    })
}
fn accepted(value: &Value) -> bool {
    value.get("success") != Some(&Value::Bool(false))
        && ["ret", "errcode", "code"].iter().all(|key| {
            value
                .get(*key)
                .is_none_or(|value| value.as_i64() == Some(0))
        })
}

async fn state_json(runtime: &Runtime, settings: &Settings) -> Value {
    let state = states()
        .lock()
        .await
        .get(&key(runtime, &settings.channel.tag))
        .cloned();
    let expired = state
        .as_ref()
        .is_some_and(|state| now().saturating_sub(state.updated_at) >= 240);
    json!({"success":true,"connected":settings.connected,"account_id":settings.account_id,"base_url":settings.base_url,"default_target":settings.channel.mp_config.get("WECHATCLAWBOT_DEFAULT_TARGET"),"known_targets":[],"qrcode":state.as_ref().map(|state| &state.qrcode),"qrcode_url":state.as_ref().map(|state| &state.url),"qrcode_status":if settings.connected { "confirmed" } else if expired { "expired" } else { state.as_ref().map_or("waiting", |state| state.status.as_str()) },"qrcode_updated_at":state.as_ref().map(|state| state.updated_at)})
}

pub async fn status(
    State(runtime): State<Arc<Runtime>>,
    Query(request): Query<TagRequest>,
) -> Response {
    let _gate = runtime.settings_gate.lock().await;
    let result = document(&runtime)
        .await
        .and_then(|(_, document)| read_settings(&document, &request.tag));
    match result {
        Ok(settings) => output(StatusCode::OK, state_json(&runtime, &settings).await),
        Err(message) => failure(StatusCode::BAD_REQUEST, &message),
    }
}

pub async fn qrcode(
    State(runtime): State<Arc<Runtime>>,
    request: Result<Json<TagRequest>, JsonRejection>,
) -> Response {
    let Ok(Json(request)) = request else {
        return failure(StatusCode::BAD_REQUEST, "二维码请求无效。");
    };
    let settings = {
        let _gate = runtime.settings_gate.lock().await;
        match document(&runtime)
            .await
            .and_then(|(_, document)| read_settings(&document, &request.tag))
        {
            Ok(settings) => settings,
            Err(message) => return failure(StatusCode::BAD_REQUEST, &message),
        }
    };
    if settings.connected {
        return output(StatusCode::OK, state_json(&runtime, &settings).await);
    }
    let value = match remote(&settings, "/ilink/bot/get_bot_qrcode", &[("bot_type", "3")]).await {
        Ok(value) => value,
        Err(message) => return failure(StatusCode::BAD_GATEWAY, &message),
    };
    if !accepted(&value) {
        return failure(StatusCode::BAD_GATEWAY, "获取微信 ClawBot 二维码失败。");
    }
    let payload = data(&value);
    let Some(qrcode) = text(payload, &["qrcode", "qr_code", "qrcode_id", "ticket"])
        .filter(|value| value.len() <= 2048)
    else {
        return failure(StatusCode::BAD_GATEWAY, "iLink 没有返回有效二维码。");
    };
    let url = text(
        payload,
        &[
            "qrcode_url",
            "url",
            "qrcodeUrl",
            "qr_url",
            "qrcode_img_content",
            "qrcode_img_url",
            "qr_img",
        ],
    )
    .unwrap_or_else(|| {
        let mut url = reqwest::Url::parse("https://liteapp.weixin.qq.com/q/7GiQu1").ok();
        if let Some(url) = url.as_mut() {
            url.query_pairs_mut()
                .append_pair("qrcode", &qrcode)
                .append_pair("bot_type", "3");
        }
        url.map_or_else(String::new, |url| url.to_string())
    });
    if url.len() > 131_072 {
        return failure(StatusCode::BAD_GATEWAY, "二维码内容过大。");
    }
    let mut states = states().lock().await;
    if states.len() >= 128 {
        states.retain(|_, state| now().saturating_sub(state.updated_at) < 240);
    }
    states.insert(
        key(&runtime, &request.tag),
        QrState {
            qrcode,
            url,
            status: "waiting".into(),
            updated_at: now(),
            base_url: settings.base_url.clone(),
        },
    );
    drop(states);
    output(StatusCode::OK, state_json(&runtime, &settings).await)
}

pub async fn poll(
    State(runtime): State<Arc<Runtime>>,
    request: Result<Json<PollRequest>, JsonRejection>,
) -> Response {
    let Ok(Json(request)) = request else {
        return failure(StatusCode::BAD_REQUEST, "二维码状态请求无效。");
    };
    let Some(issued) = states()
        .lock()
        .await
        .get(&key(&runtime, &request.tag))
        .cloned()
    else {
        return failure(StatusCode::BAD_REQUEST, "请先刷新二维码。");
    };
    if issued.qrcode != request.qrcode || now().saturating_sub(issued.updated_at) >= 240 {
        return failure(StatusCode::CONFLICT, "二维码已过期，请刷新后重试。");
    }
    let settings = {
        let _gate = runtime.settings_gate.lock().await;
        match document(&runtime)
            .await
            .and_then(|(_, document)| read_settings(&document, &request.tag))
        {
            Ok(settings) => settings,
            Err(message) => return failure(StatusCode::BAD_REQUEST, &message),
        }
    };
    if settings.connected {
        return output(StatusCode::OK, state_json(&runtime, &settings).await);
    }
    if settings.base_url != issued.base_url {
        return failure(StatusCode::CONFLICT, "iLink 地址已修改，请刷新二维码。");
    }
    let value = match remote(
        &settings,
        "/ilink/bot/get_qrcode_status",
        &[("qrcode", &request.qrcode)],
    )
    .await
    {
        Ok(value) => value,
        Err(message) => return failure(StatusCode::BAD_GATEWAY, &message),
    };
    if !accepted(&value) {
        return failure(StatusCode::BAD_GATEWAY, "查询微信 ClawBot 二维码状态失败。");
    }
    let payload = data(&value);
    let state = text(payload, &["status", "state", "scan_status"])
        .unwrap_or_else(|| "waiting".into())
        .to_lowercase();
    let token = text(
        payload,
        &["bot_token", "token", "access_token", "jwt", "auth_token"],
    );
    if let Some(token) = token {
        let Some(account) = text(
            payload,
            &[
                "account_id",
                "ilink_bot_id",
                "wxid",
                "uid",
                "user_id",
                "from_user",
                "from_uid",
            ],
        ) else {
            return failure(StatusCode::BAD_GATEWAY, "iLink 登录响应缺少账号ID。");
        };
        let base_url = text(payload, &["baseurl", "base_url"])
            .or_else(|| text(&value, &["baseurl", "base_url"]))
            .unwrap_or(settings.base_url.clone());
        if token.len() > 65536 || account.len() > 512 || validate_url(&base_url).is_err() {
            return failure(StatusCode::BAD_GATEWAY, "iLink 登录响应无效。");
        }
        let _gate = runtime.settings_gate.lock().await;
        let (bytes, mut document) = match document(&runtime).await {
            Ok(document) => document,
            Err(message) => return failure(StatusCode::BAD_REQUEST, &message),
        };
        let current = match read_settings(&document, &request.tag) {
            Ok(current) => current,
            Err(message) => return failure(StatusCode::BAD_REQUEST, &message),
        };
        if current.base_url != issued.base_url {
            return failure(StatusCode::CONFLICT, "通知配置已修改，请重新扫码。");
        }
        if let Err(message) = store_token(
            &mut document,
            &request.tag,
            &current.token_key,
            Some((&token, &account, &base_url)),
        ) {
            return failure(StatusCode::BAD_REQUEST, &message);
        }
        let contents = match serde_yaml::to_string(&document) {
            Ok(contents) => contents,
            Err(_) => return failure(StatusCode::INTERNAL_SERVER_ERROR, "无法编码通知配置。"),
        };
        if config::replace_document(&runtime.config.source_path, &bytes, &contents)
            .await
            .is_err()
        {
            return failure(
                StatusCode::CONFLICT,
                "配置已修改或无法写入，请重新加载后重试。",
            );
        }
        if let Some(issued) = states().lock().await.get_mut(&key(&runtime, &request.tag)) {
            issued.status = "confirmed".into();
        }
        return output(
            StatusCode::OK,
            json!({"success":true,"connected":true,"account_id":account,"qrcode_status":"confirmed","restart_required":true,"message":"微信 ClawBot 已登录，重启服务后使用新凭据发送通知。"}),
        );
    }
    if let Some(issued) = states().lock().await.get_mut(&key(&runtime, &request.tag)) {
        issued.status = state;
    }
    output(StatusCode::OK, state_json(&runtime, &settings).await)
}

fn store_token(
    document: &mut serde_yaml::Value,
    tag: &str,
    reference: &str,
    login: Option<(&str, &str, &str)>,
) -> Result<(), String> {
    let notifications = document
        .get_mut("notifications")
        .and_then(serde_yaml::Value::as_mapping_mut)
        .ok_or("通知配置无效。")?;
    let channels = notifications
        .get_mut("channels")
        .and_then(serde_yaml::Value::as_sequence_mut)
        .ok_or("通知渠道列表无效。")?;
    let shared = channels
        .iter()
        .filter(|channel| channel["bot_token_env"].as_str() == Some(reference))
        .count()
        > 1;
    let token_key = if reference.is_empty() || shared {
        format!(
            "NOTIFY_{}_CLAWBOT_TOKEN",
            tag.replace('-', "_").to_uppercase()
        )
    } else {
        reference.to_string()
    };
    let channel = channels
        .iter_mut()
        .find(|channel| channel["tag"].as_str() == Some(tag))
        .and_then(serde_yaml::Value::as_mapping_mut)
        .ok_or("通知渠道不存在。")?;
    channel.insert("bot_token_env".into(), token_key.clone().into());
    if let Some((_, account, base_url)) = login {
        channel.insert("account_id".into(), account.into());
        channel.insert("api_url".into(), base_url.into());
        let mp = channel
            .entry("mp_config".into())
            .or_insert_with(|| serde_yaml::Value::Mapping(serde_yaml::Mapping::new()))
            .as_mapping_mut()
            .ok_or("MP 通知配置格式无效。")?;
        mp.insert("WECHATCLAWBOT_BASE_URL".into(), base_url.into());
    }
    let secrets = notifications
        .entry("secrets".into())
        .or_insert_with(|| serde_yaml::Value::Mapping(serde_yaml::Mapping::new()))
        .as_mapping_mut()
        .ok_or("通知凭据配置格式无效。")?;
    secrets.insert(
        token_key.into(),
        login.map_or("", |(token, _, _)| token).into(),
    );
    Ok(())
}

pub async fn logout(
    State(runtime): State<Arc<Runtime>>,
    request: Result<Json<TagRequest>, JsonRejection>,
) -> Response {
    let Ok(Json(request)) = request else {
        return failure(StatusCode::BAD_REQUEST, "退出登录请求无效。");
    };
    let _gate = runtime.settings_gate.lock().await;
    let (bytes, mut document) = match document(&runtime).await {
        Ok(document) => document,
        Err(message) => return failure(StatusCode::BAD_REQUEST, &message),
    };
    let settings = match read_settings(&document, &request.tag) {
        Ok(settings) => settings,
        Err(message) => return failure(StatusCode::BAD_REQUEST, &message),
    };
    if std::env::var(&settings.token_key)
        .ok()
        .is_some_and(|value| !value.trim().is_empty())
    {
        return failure(
            StatusCode::CONFLICT,
            "Token 来自环境变量，请先在容器设置中清除该变量。",
        );
    }
    if let Err(message) = store_token(&mut document, &request.tag, &settings.token_key, None) {
        return failure(StatusCode::BAD_REQUEST, &message);
    }
    let contents = match serde_yaml::to_string(&document) {
        Ok(contents) => contents,
        Err(_) => return failure(StatusCode::INTERNAL_SERVER_ERROR, "无法编码通知配置。"),
    };
    if config::replace_document(&runtime.config.source_path, &bytes, &contents)
        .await
        .is_err()
    {
        return failure(
            StatusCode::CONFLICT,
            "配置已修改或无法写入，请重新加载后重试。",
        );
    }
    states().lock().await.remove(&key(&runtime, &request.tag));
    output(
        StatusCode::OK,
        json!({"success":true,"connected":false,"restart_required":true,"message":"已清除微信 ClawBot 登录凭据，重启服务后生效。"}),
    )
}
