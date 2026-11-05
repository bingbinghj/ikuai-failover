use std::{collections::HashMap, future::IntoFuture, sync::Arc, time::Duration};

use axum::{
    Router,
    body::Bytes,
    extract::State,
    http::{HeaderMap, Method, StatusCode, Uri},
    response::IntoResponse,
    routing::any,
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use ikb_failover::notification::{Event, NotificationConfig, NotificationError, Notifier};
use serde_json::{Value, json};
use tokio::{sync::Mutex, task::JoinHandle};

#[derive(Clone)]
struct MockState {
    requests: Arc<Mutex<Vec<RecordedRequest>>>,
    rejection: bool,
    delay_ms: u64,
}

struct RecordedRequest {
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Vec<u8>,
}

async fn receive(
    State(state): State<MockState>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    state.requests.lock().await.push(RecordedRequest {
        method,
        uri: uri.clone(),
        headers,
        body: body.to_vec(),
    });
    tokio::time::sleep(Duration::from_millis(state.delay_ms)).await;
    let mut value = if state.rejection {
        json!({"ok":false,"success":false,"errcode":401,"code":401,"ret":401,"error":{"code":401},"accepted":0,"failed":["mock-device"]})
    } else {
        json!({"ok":true,"success":true,"id":"mock-message","errcode":0,"code":0,"ret":0,"access_token":"mock-issued-token","tenant_access_token":"mock-issued-token","accepted":1,"failed":[]})
    };
    if !state.rejection && (uri.path().contains("discord") || uri.path().contains("/qq/v2/")) {
        value.as_object_mut().unwrap().remove("code");
    }
    let status =
        if state.rejection && (uri.path().contains("voce") || uri.path().contains("webpush")) {
            StatusCode::BAD_REQUEST
        } else {
            StatusCode::OK
        };
    (status, axum::Json(value))
}

async fn server(rejection: bool, delay_ms: u64) -> (String, MockState, JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let state = MockState {
        requests: Arc::new(Mutex::new(Vec::new())),
        rejection,
        delay_ms,
    };
    let router = Router::new()
        .fallback(any(receive))
        .with_state(state.clone());
    let task = tokio::spawn(async move {
        let _ = tokio::time::timeout(
            Duration::from_secs(60),
            axum::serve(listener, router).into_future(),
        )
        .await;
    });
    (format!("http://{address}"), state, task)
}

fn event() -> Event {
    Event {
        id: "mock-event-1".into(),
        kind: "wan_switched".into(),
        title: "网络出口切换".into(),
        text: "wan1 → wan3".into(),
        timestamp_ms: 123456789,
        context: Default::default(),
        previous_context: None,
    }
}

fn notifier(value: Value, secrets: &HashMap<String, String>) -> Notifier {
    let config: NotificationConfig = serde_json::from_value(value).unwrap();
    config.validate().unwrap();
    Notifier::new_with_env(&config, |key| {
        secrets
            .get(key)
            .cloned()
            .ok_or(NotificationError::SecretMissing)
    })
    .unwrap()
}

fn channels(base: &str) -> Vec<Value> {
    vec![
        json!({"tag":"telegram","enabled":true,"provider":"telegram","api_url":format!("{base}/telegram"),"bot_token_env":"MOCK_SECRET","chat_id":"-123","message_thread_id":42}),
        json!({"tag":"slack","enabled":true,"provider":"slack","api_url":format!("{base}/slack"),"bot_token_env":"MOCK_SECRET","channel":"C123"}),
        json!({"tag":"discord","enabled":true,"provider":"discord","api_url":format!("{base}/discord"),"bot_token_env":"MOCK_SECRET","channel_id":"123"}),
        json!({"tag":"dingtalk","enabled":true,"provider":"ding_talk","webhook_url_env":"MOCK_DINGTALK_URL","secret_env":"MOCK_SECRET"}),
        json!({"tag":"synology","enabled":true,"provider":"synology_chat","webhook_url_env":"MOCK_SYNOLOGY_URL","user_ids":[1,2]}),
        json!({"tag":"voce","enabled":true,"provider":"voce_chat","api_url":format!("{base}/voce"),"api_key_env":"MOCK_SECRET","target":"GID#123"}),
        json!({"tag":"wechat","enabled":true,"provider":"wechat","api_url":format!("{base}/wechat"),"corp_id":"mock-corp","corp_secret_env":"MOCK_SECRET","agent_id":1,"touser":"mock-user"}),
        json!({"tag":"feishu","enabled":true,"provider":"feishu","api_url":format!("{base}/feishu"),"app_id":"mock-app","app_secret_env":"MOCK_SECRET","receive_id":"mock-chat","receive_id_type":"chat_id"}),
        json!({"tag":"clawbot","enabled":true,"provider":"wechat_claw_bot","api_url":format!("{base}/clawbot"),"bot_token_env":"MOCK_SECRET","account_id":"mock-account","recipient":"mock-user","context_token_env":"MOCK_SECRET"}),
        json!({"tag":"qq","enabled":true,"provider":"qq","api_url":format!("{base}/qq"),"token_url":format!("{base}/qq-token"),"app_id":"mock-app","app_secret_env":"MOCK_SECRET","recipient":"mock-user","recipient_type":"group"}),
        json!({"tag":"webpush","enabled":true,"provider":"web_push","subscriptions_env":"MOCK_SUBSCRIPTIONS","private_key_env":"MOCK_PRIVATE_KEY","subject":"mailto:test@example.invalid"}),
        json!({"tag":"apppush","enabled":true,"provider":"app_push","api_url":format!("{base}/apppush"),"relay_key_env":"MOCK_RELAY_KEY","owner":"mock-owner"}),
        json!({"tag":"web","enabled":true,"provider":"web","receive_url_env":"MOCK_WEB_URL","token_env":"MOCK_SECRET"}),
        json!({"tag":"webagent","enabled":true,"provider":"web_agent","receive_url_env":"MOCK_WEBAGENT_URL","token_env":"MOCK_SECRET"}),
    ]
}

fn secrets(
    base: &str,
) -> (
    HashMap<String, String>,
    web_push_native::p256::SecretKey,
    [u8; 16],
) {
    use web_push_native::{
        jwt_simple::algorithms::ES256KeyPair,
        p256::{
            SecretKey,
            elliptic_curve::{rand_core::OsRng, sec1::ToEncodedPoint},
        },
    };
    let private = ES256KeyPair::generate();
    let subscriber = SecretKey::random(&mut OsRng);
    let public = subscriber.public_key().to_encoded_point(false);
    let auth = [9u8; 16];
    let subscriptions = json!([{"endpoint":format!("{base}/webpush"),"keys":{"p256dh":URL_SAFE_NO_PAD.encode(public.as_bytes()),"auth":URL_SAFE_NO_PAD.encode(auth)}}]);
    let secrets = HashMap::from([
        ("MOCK_SECRET".into(), "mock-secret".into()),
        (
            "MOCK_DINGTALK_URL".into(),
            format!("{base}/dingtalk?access_token=mock-token"),
        ),
        (
            "MOCK_SYNOLOGY_URL".into(),
            format!("{base}/synology?token=mock-token"),
        ),
        ("MOCK_WEB_URL".into(), format!("{base}/web")),
        ("MOCK_WEBAGENT_URL".into(), format!("{base}/webagent")),
        ("MOCK_SUBSCRIPTIONS".into(), subscriptions.to_string()),
        (
            "MOCK_PRIVATE_KEY".into(),
            URL_SAFE_NO_PAD.encode(private.to_bytes()),
        ),
        (
            "MOCK_RELAY_KEY".into(),
            "mock-relay-key-long-enough-0123456789".into(),
        ),
    ]);
    (secrets, subscriber, auth)
}

#[tokio::test]
async fn all_moviepilot_channels_send_their_actual_protocols_to_local_mocks() {
    let (base, state, server) = server(false, 0).await;
    let (secrets, subscriber, auth) = secrets(&base);
    let notifier = notifier(json!({"enabled":true,"channels":channels(&base)}), &secrets);
    let results = notifier.send(&event()).await;
    assert_eq!(results.len(), 14);
    assert!(results.iter().all(|result| result.success), "{results:?}");
    let requests = state.requests.lock().await;
    let find = |path: &str| {
        requests
            .iter()
            .find(|request| request.uri.path() == path)
            .unwrap()
    };
    let telegram = find("/telegram/botmock-secret/sendMessage");
    let body: Value = serde_json::from_slice(&telegram.body).unwrap();
    assert_eq!(body["chat_id"], "-123");
    assert_eq!(body["message_thread_id"], 42);
    assert_eq!(body["parse_mode"], "MarkdownV2");
    assert!(serde_json::from_slice::<Value>(&find("/discord/channels/123/messages").body).unwrap()["embeds"].is_array());
    let discord = find("/discord/channels/123/messages");
    assert_eq!(discord.headers["authorization"], "Bot mock-secret");
    assert_eq!(
        serde_json::from_slice::<Value>(&discord.body).unwrap()["allowed_mentions"]["parse"],
        json!([])
    );
    assert_eq!(
        find("/slack/chat.postMessage").headers["authorization"],
        "Bearer mock-secret"
    );
    let dingtalk = find("/dingtalk");
    let query = reqwest::Url::parse(&format!("{base}{}", dingtalk.uri))
        .unwrap()
        .query_pairs()
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect::<HashMap<_, _>>();
    use hmac::{Hmac, Mac};
    let mut hmac = Hmac::<sha2::Sha256>::new_from_slice(b"mock-secret").unwrap();
    hmac.update(format!("{}\nmock-secret", query["timestamp"]).as_bytes());
    assert_eq!(
        query["sign"],
        base64::engine::general_purpose::STANDARD.encode(hmac.finalize().into_bytes())
    );
    let synology = find("/synology");
    let form = reqwest::Url::parse(&format!(
        "http://localhost/?{}",
        String::from_utf8_lossy(&synology.body)
    ))
    .unwrap()
    .query_pairs()
    .into_owned()
    .collect::<HashMap<_, _>>();
    assert_eq!(
        serde_json::from_str::<Value>(&form["payload"]).unwrap()["user_ids"],
        json!([1, 2])
    );
    assert_eq!(
        find("/voce/api/bot/send_to_group/123").headers["x-api-key"],
        "mock-secret"
    );
    assert_eq!(find("/wechat/cgi-bin/gettoken").method, Method::GET);
    assert!(
        find("/wechat/cgi-bin/message/send")
            .uri
            .query()
            .unwrap()
            .contains("mock-issued-token")
    );
    assert_eq!(
        find("/feishu/open-apis/im/v1/messages").headers["authorization"],
        "Bearer mock-issued-token"
    );
    let clawbot = find("/clawbot/ilink/bot/sendmessage");
    assert_eq!(clawbot.headers["authorizationtype"], "ilink_bot_token");
    assert_eq!(
        serde_json::from_slice::<Value>(&clawbot.body).unwrap()["msg"]["context_token"],
        "mock-secret"
    );
    assert_eq!(
        find("/qq/v2/groups/mock-user/messages").headers["authorization"],
        "QQBot mock-issued-token"
    );
    let encrypted = find("/webpush");
    assert_eq!(encrypted.headers["content-encoding"], "aes128gcm");
    assert!(
        encrypted.headers["authorization"]
            .to_str()
            .unwrap()
            .starts_with("vapid ")
    );
    let decoded = web_push_native::decrypt(
        encrypted.body.clone(),
        &subscriber,
        &web_push_native::Auth::from(auth),
    )
    .unwrap();
    assert_eq!(
        serde_json::from_slice::<Value>(&decoded).unwrap()["title"],
        "🔔 网络出口切换"
    );
    assert_eq!(
        serde_json::from_slice::<Value>(&find("/apppush/push/messages").body).unwrap()["owner"],
        "mock-owner"
    );
    assert_eq!(
        serde_json::from_slice::<Value>(&find("/web").body).unwrap()["channel"],
        "web"
    );
    assert_eq!(
        serde_json::from_slice::<Value>(&find("/webagent").body).unwrap()["channel"],
        "web_agent"
    );
    drop(requests);
    let retry = notifier.send_to(&event(), &["telegram".into()]).await;
    assert_eq!(retry.len(), 1);
    assert_eq!(retry[0].tag, "telegram");
    server.abort();
}

#[tokio::test]
async fn provider_rejections_never_become_success_or_expose_secret_values() {
    let (base, state, server) = server(true, 0).await;
    let (secrets, _, _) = secrets(&base);
    let notifier = notifier(json!({"enabled":true,"channels":channels(&base)}), &secrets);
    let results = notifier.send(&event()).await;
    assert_eq!(results.len(), 14);
    assert!(results.iter().all(|result| !result.success));
    let text = serde_json::to_string(&results).unwrap();
    assert!(!text.contains("mock-secret"));
    assert!(!text.contains(&base));
    assert!(!text.contains("access_token"));
    assert!(!state.requests.lock().await.is_empty());
    server.abort();
}

#[tokio::test]
async fn slow_provider_is_bounded_and_disabled_channels_never_resolve_secrets() {
    let (base, _, server) = server(false, 500).await;
    let (secrets, _, _) = secrets(&base);
    let notifier = notifier(
        json!({"enabled":true,"timeout_ms":100,"channels":[channels(&base)[0].clone()]}),
        &secrets,
    );
    let results = tokio::time::timeout(Duration::from_secs(1), notifier.send(&event()))
        .await
        .unwrap();
    assert_eq!(results[0].error_code.as_deref(), Some("timeout"));
    let config: NotificationConfig =
        serde_json::from_value(json!({"enabled":false,"channels":[channels(&base)[0].clone()]}))
            .unwrap();
    let disabled = Notifier::new_with_env(&config, |_| {
        panic!("Disabled notifications must not read credentials")
    })
    .unwrap();
    assert!(disabled.send(&event()).await.is_empty());
    server.abort();
}

#[test]
fn configuration_checks_are_offline_and_missing_bridge_endpoint_is_rejected() {
    let config:NotificationConfig=serde_json::from_value(json!({"enabled":true,"channels":[{"tag":"telegram","enabled":true,"provider":"telegram","bot_token_env":"MOCK_MISSING","chat_id":"1"}]})).unwrap();
    assert!(config.validate().is_ok());
    assert!(Notifier::new_with_env(&config, |_| Err(NotificationError::SecretMissing)).is_ok());
    let invalid=serde_json::from_value::<NotificationConfig>(json!({"enabled":true,"channels":[{"tag":"web","enabled":true,"provider":"web","receive_url_env":""}]})).unwrap();
    assert!(invalid.validate().is_err());
}

#[tokio::test]
async fn missing_credentials_are_isolated_to_their_channel_and_optional_secrets_can_be_absent() {
    let (base, _, server) = server(false, 0).await;
    let mut channels = channels(&base);
    channels[0]["bot_token_env"] = json!("MOCK_MISSING");
    let (mut secrets, _, _) = secrets(&base);
    secrets.remove("MOCK_SECRET");
    secrets.insert("MOCK_SLACK_SECRET".into(), "mock-secret".into());
    channels[1]["bot_token_env"] = json!("MOCK_SLACK_SECRET");
    let notifier = notifier(
        json!({"enabled":true,"channels":[channels[0].clone(),channels[1].clone(),channels[3].clone()]}),
        &secrets,
    );
    let results = notifier.send(&event()).await;
    assert_eq!(results.len(), 3);
    let missing = results
        .iter()
        .find(|result| result.tag == "telegram")
        .unwrap();
    assert_eq!(missing.error_code.as_deref(), Some("secret_missing"));
    assert!(
        results
            .iter()
            .filter(|result| result.tag != "telegram")
            .all(|result| result.success)
    );
    server.abort();
}
