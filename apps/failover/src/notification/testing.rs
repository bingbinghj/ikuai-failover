use super::{Event, NotificationConfig, Notifier, templates};
use crate::runtime::Runtime;
use axum::{
    Json,
    extract::{State, rejection::JsonRejection},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
};
use serde::Deserialize;
use serde_json::json;
use std::{
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TestRequest {
    notifications: NotificationConfig,
    tag: String,
    #[serde(default = "default_kind")]
    kind: String,
    template: Option<String>,
    title: Option<String>,
    text: Option<String>,
    message_format: Option<String>,
}

fn default_kind() -> String {
    "notification_test".into()
}

fn sample(kind: String) -> Event {
    let timestamp_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|time| time.as_millis() as u64)
        .unwrap_or_default();
    Event {
        id: format!("test-{timestamp_ms}"),
        kind,
        title: "通知测试".into(),
        text: "这是一条来自 ikuai-failover 的测试消息，收到此消息说明通知渠道已连接。".into(),
        timestamp_ms,
        context: Default::default(),
        previous_context: None,
    }
}

pub async fn send_test(
    State(runtime): State<Arc<Runtime>>,
    request: Result<Json<TestRequest>, JsonRejection>,
) -> Response {
    let Ok(Json(mut request)) = request else {
        return failure("请检查通知渠道配置。");
    };
    let Some(channel) = request
        .notifications
        .channels
        .iter()
        .find(|channel| channel.tag == request.tag)
        .cloned()
    else {
        return failure("请选择需要测试的通知渠道。");
    };
    request.notifications.channels = vec![channel];
    request.notifications.enabled = true;
    request.notifications.channels[0].enabled = true;
    if let Some(format) = request.message_format {
        request.notifications.channels[0].message_format = Some(format);
    }
    if let Some(template) = request.template {
        if templates::validate(&template).is_err() {
            return failure("通知模板格式有误，请先预览模板。");
        }
        request
            .notifications
            .templates
            .insert(templates::category(&request.kind).into(), template);
    }
    let Ok(notifier) = Notifier::new(&request.notifications) else {
        return failure("通知配置不完整，请检查必填项和通知模板。");
    };
    let mut event = sample(request.kind);
    if let Some(title) = request.title {
        event.title = title;
    }
    if let Some(text) = request.text {
        event.text = text;
    }
    event.context = runtime
        .events
        .test_network_context(
            super::network_context::NetworkContext::from_snapshot(&runtime.snapshot().await),
            &event.kind,
        )
        .await;
    event.previous_context = event.context.previous.as_deref().cloned();
    let results = notifier.send_to(&event, &[request.tag]).await;
    let success = !results.is_empty() && results.iter().all(|result| result.success);
    if success {
        crate::logging::info("[通知测试] 测试消息已发送。");
    } else {
        crate::logging::warn("[通知测试] 测试发送未成功，请检查渠道配置和网络连接。");
    }
    // Draft values and provider error bodies never enter service logs.
    let _ = runtime;
    no_store(Json(json!({"success":success,"deliveries":results,"message":if success {"测试消息已发送。"} else {"发送失败，请检查渠道配置、密钥及网络连接。"}})).into_response())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreviewRequest {
    template: String,
    #[serde(default = "default_kind")]
    kind: String,
    title: Option<String>,
    text: Option<String>,
    context: Option<super::network_context::NetworkContext>,
    previous_context: Option<super::network_context::NetworkContext>,
}

pub async fn preview(
    State(runtime): State<Arc<Runtime>>,
    request: Result<Json<PreviewRequest>, JsonRejection>,
) -> Response {
    let Ok(Json(request)) = request else {
        return failure("请填写有效的通知模板。");
    };
    let mut event = sample(request.kind);
    event.context = request
        .context
        .unwrap_or_else(super::network_context::NetworkContext::default);
    if event.context.interface.is_empty() {
        event.context =
            super::network_context::NetworkContext::from_snapshot(&runtime.snapshot().await);
    }
    event.previous_context = request.previous_context;
    if event.previous_context.is_none() {
        event.context = runtime
            .events
            .test_network_context(event.context, &event.kind)
            .await;
        event.previous_context = event.context.previous.as_deref().cloned();
    }
    if let Some(title) = request.title {
        event.title = title;
    }
    if let Some(text) = request.text {
        event.text = text;
    }
    match templates::render(&request.template, &event) {
        Ok(event) => no_store(
            Json(json!({"success":true,"title":event.title,"text":event.text})).into_response(),
        ),
        Err(_) => failure("模板格式或变量有误，请使用包含 title 和 text 的 JSON 对象。"),
    }
}

pub async fn defaults() -> Response {
    no_store(Json(json!({"templates":templates::defaults()})).into_response())
}

fn failure(message: &str) -> Response {
    no_store(
        (
            StatusCode::BAD_REQUEST,
            Json(json!({"success":false,"message":message})),
        )
            .into_response(),
    )
}
fn no_store(mut response: Response) -> Response {
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        axum::http::HeaderValue::from_static("no-store"),
    );
    response
}
