use super::{
    Event, NotificationConfig, ProviderConfig, network_context::NetworkContext, providers,
    transport::PreparedChannel,
};
use axum::{
    Json, Router,
    extract::{OriginalUri, State},
    http::StatusCode,
    routing::post,
};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, HashMap},
    sync::{Arc, Mutex},
};

fn event() -> Event {
    Event {
        id: "format-regression".into(),
        kind: "egress_switched".into(),
        title: "**出口切换**".into(),
        text: format!(
            "[控制台](https://example.com/panel)\n{}",
            "**正文**\n".repeat(3000)
        ),
        timestamp_ms: 1_799_000_000_000,
        context: NetworkContext {
            interface: "WAN1".into(),
            isp: "中国联通".into(),
            ipv4: "203.0.113.10".into(),
            ipv6: "2408:abcd::1".into(),
            ..Default::default()
        },
        previous_context: Some(NetworkContext {
            interface: "WAN3".into(),
            isp: "中国移动".into(),
            ipv4: "198.51.100.20".into(),
            ipv6: "2001:db8::20".into(),
            ..Default::default()
        }),
    }
}

#[derive(Clone)]
struct Mock {
    requests: Arc<Mutex<Vec<Value>>>,
    rejection: i64,
}

async fn receive(
    State(state): State<Mock>,
    OriginalUri(uri): OriginalUri,
    body: Option<Json<Value>>,
) -> (StatusCode, Json<Value>) {
    let body = body.map(|body| body.0).unwrap_or(Value::Null);
    if uri.path().ends_with("gettoken") || uri.path().ends_with("token") {
        return (
            StatusCode::OK,
            Json(json!({"errcode":0,"access_token":"mock"})),
        );
    }
    state.requests.lock().unwrap().push(body.clone());
    if state.rejection != 0 && body["msgtype"] == "news" {
        return (StatusCode::OK, Json(json!({"errcode":state.rejection})));
    }
    if state.rejection != 0 && body["msg_type"] == 2 {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"code":state.rejection})),
        );
    }
    (
        StatusCode::OK,
        Json(json!({"errcode":0,"id":"mock-id","ok":true})),
    )
}

async fn server(rejection: i64) -> (String, Mock, tokio::task::JoinHandle<()>) {
    let state = Mock {
        requests: Arc::new(Mutex::new(Vec::new())),
        rejection,
    };
    let router = Router::new()
        .fallback(post(receive).get(receive))
        .with_state(state.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    (base, state, server)
}

fn channel(base: &str, format: &str) -> PreparedChannel {
    PreparedChannel {
        provider: ProviderConfig::Wechat {
            api_url: base.into(),
            corp_id: "corp".into(),
            corp_secret_env: "SECRET".into(),
            agent_id: 1,
            touser: "user".into(),
        },
        secrets: HashMap::from([("SECRET".into(), "mock".into())]),
        mp_config: BTreeMap::new(),
        message_format: format.into(),
    }
}

#[test]
fn news_keeps_transition_and_utf8_limits_without_images_or_links() {
    let mut event = event();
    event.title = "中".repeat(100);
    let article = super::wechat::article(&event).unwrap();
    assert!(article["title"].as_str().unwrap().len() <= 128);
    let description = article["description"].as_str().unwrap();
    assert!(description.len() <= 512);
    for value in [
        "WAN1",
        "WAN3",
        "中国联通",
        "中国移动",
        "203.0.113.10",
        "198.51.100.20",
        "2408:abcd::1",
        "2001:db8::20",
        "时间",
    ] {
        assert!(
            description.contains(value),
            "missing {value}: {description}"
        );
    }
    assert!(!description.contains("**"));
    assert!(article["url"].is_null());
    assert!(article.get("picurl").is_none());
    event
        .context
        .lines
        .push(super::network_context::LineContext {
            interface: "WAN2".into(),
            isp: "中国电信".into(),
            ipv4: "192.0.2.10".into(),
            ..Default::default()
        });
    let description = super::wechat::article(&event).unwrap()["description"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(description.contains("WAN2"));
    assert!(description.contains("192.0.2.10"));
}

#[tokio::test]
async fn news_retries_content_rejection_but_not_authentication_failures() {
    let client = super::transport::client(5000).unwrap();
    for (rejection, expected_success, expected_count) in [
        (0, true, 1),
        (40058, true, 2),
        (40033, true, 2),
        (40014, false, 1),
    ] {
        let (base, state, task) = server(rejection).await;
        let result = providers::send(&client, &channel(&base, "news"), &event()).await;
        assert_eq!(result.is_ok(), expected_success, "code {rejection}");
        let requests = state.requests.lock().unwrap();
        assert_eq!(requests.len(), expected_count);
        assert_eq!(requests[0]["msgtype"], "news");
        if expected_count == 2 {
            assert_eq!(requests[1]["msgtype"], "text");
            let content = requests[1]["text"]["content"].as_str().unwrap();
            assert!(content.len() <= 2048);
            assert!(content.contains("2408:abcd::1"));
        }
        task.abort();
    }
}

#[tokio::test]
async fn application_native_markdown_obeys_2048_byte_limit() {
    let (base, state, task) = server(0).await;
    let mut channel = channel(&base, "markdown");
    channel
        .mp_config
        .insert("WECHAT_APP_MARKDOWN".into(), json!(true));
    providers::send(&super::transport::client(5000).unwrap(), &channel, &event())
        .await
        .unwrap();
    let requests = state.requests.lock().unwrap();
    assert_eq!(requests[0]["msgtype"], "markdown");
    let content = requests[0]["markdown"]["content"].as_str().unwrap();
    assert!(content.len() <= 2048);
    assert!(content.contains("2408:abcd::1"));
    task.abort();
}

#[test]
fn news_is_restricted_to_application_channels() {
    for (provider, mode, accepted) in [
        ("wechat", "app", true),
        ("wechat", "bot", false),
        ("telegram", "app", false),
    ] {
        let config: NotificationConfig = serde_json::from_value(json!({"channels":[{"tag":"check","provider":provider,"corp_id":"corp","corp_secret_env":"SECRET","agent_id":1,"bot_token_env":"SECRET","chat_id":"user","message_format":"news","mp_config":{"WECHAT_MODE":mode}}]})).unwrap();
        assert_eq!(config.validate().is_ok(), accepted);
    }
}

#[tokio::test]
async fn qq_native_permission_rejection_falls_back_on_http_400_only_for_known_code() {
    let client = super::transport::client(5000).unwrap();
    for (code, success, count) in [(11244, true, 2), (401, false, 1)] {
        let (base, state, task) = server(code).await;
        let mut channel = channel(&base, "markdown");
        channel.provider = ProviderConfig::Qq {
            api_url: base.clone(),
            token_url: format!("{base}/token"),
            app_id: "app".into(),
            app_secret_env: "SECRET".into(),
            recipient: "user".into(),
            recipient_type: "user".into(),
        };
        channel.mp_config.insert("QQ_MARKDOWN".into(), json!(true));
        let result = providers::send(&client, &channel, &event()).await;
        assert_eq!(result.is_ok(), success);
        let requests = state.requests.lock().unwrap();
        assert_eq!(requests.len(), count);
        if count == 2 {
            assert_eq!(requests[1]["msg_type"], 0);
            assert!(!requests[1]["content"].as_str().unwrap().contains("**"));
        }
        task.abort();
    }
}

#[tokio::test]
async fn template_test_format_override_changes_only_the_selected_draft_delivery() {
    let directory = tempfile::tempdir().unwrap();
    let mut config: crate::config::Config =
        serde_yaml::from_str(include_str!("../../../../config.example.yml")).unwrap();
    config.wan.enabled = false;
    config.management.state_dir = directory.path().join("state");
    let runtime = Arc::new(
        crate::runtime::Runtime::new(config, Arc::new(crate::api::DisabledClient))
            .await
            .unwrap(),
    );
    let before = serde_json::to_value(&runtime.config.notifications).unwrap();
    let app = Router::new()
        .route("/notifications/test", post(super::testing::send_test))
        .with_state(runtime.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let test_url = format!(
        "http://{}/notifications/test",
        listener.local_addr().unwrap()
    );
    let api_task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let (base, state, task) = server(0).await;
    let client = reqwest::Client::new();
    for (provider, format, expected_key, expected) in [
        ("telegram", "html", "parse_mode", "HTML"),
        ("wechat", "news", "msgtype", "news"),
        ("discord", "embed", "embeds", ""),
    ] {
        let config = json!({"channels":[{"tag":"draft","enabled":true,"provider":provider,"api_url":base,"corp_id":"corp","corp_secret_env":"SECRET","agent_id":1,"touser":"user","bot_token_env":"SECRET","chat_id":"user","channel_id":"123","message_format":"text"}],"secrets":{"SECRET":"mock"}});
        let response = client.post(&test_url).json(&json!({"notifications":config,"tag":"draft","template":"{}","title":"**草稿**","text":"<b>格式测试</b>","message_format":format})).send().await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.json::<Value>().await.unwrap()["success"], true);
        let payload = state.requests.lock().unwrap().pop().unwrap();
        if expected.is_empty() {
            assert!(payload[expected_key].is_array());
        } else {
            assert_eq!(payload[expected_key], expected);
        }
        assert_eq!(
            serde_json::to_value(&runtime.config.notifications).unwrap(),
            before
        );
    }
    let invalid = client.post(&test_url).json(&json!({"notifications":{"channels":[{"tag":"draft","provider":"telegram","bot_token_env":"SECRET","chat_id":"user"}]},"tag":"draft","message_format":"news"})).send().await.unwrap();
    assert_eq!(invalid.status(), StatusCode::BAD_REQUEST);
    assert!(state.requests.lock().unwrap().is_empty());
    api_task.abort();
    task.abort();
}
