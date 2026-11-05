use std::{
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use axum::{Json, Router, middleware, routing::get};
use base64::Engine;
use ikb_failover::auth::{self, AuthState, WebUiAuthConfig};
use reqwest::{StatusCode, header};
use serde_json::{Value, json};

async fn server(user: &str, pass: &str) -> (String, tokio::task::JoinHandle<()>) {
    let state = AuthState::new(WebUiAuthConfig {
        user: user.into(),
        pass: pass.into(),
        ..Default::default()
    })
    .await;
    server_with_state(state).await
}

async fn server_with_state(state: Arc<AuthState>) -> (String, tokio::task::JoinHandle<()>) {
    let protected = Router::new()
        .route(
            "/status",
            get(|| async { Json(json!({"ok":true})) }).post(|| async { Json(json!({"ok":true})) }),
        )
        .route_layer(middleware::from_fn_with_state(state.clone(), auth::protect));
    let app = protected.merge(auth::routes(state));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let task = tokio::spawn(async move {
        tokio::time::timeout(Duration::from_secs(10), axum::serve(listener, app))
            .await
            .unwrap()
            .unwrap();
    });
    (format!("http://{address}"), task)
}

fn basic(value: &str) -> String {
    format!(
        "Basic {}",
        base64::engine::general_purpose::STANDARD.encode(value)
    )
}

#[tokio::test]
async fn original_basic_auth_matches_raw_credentials_and_rejects_invalid_headers() {
    let (url, task) = server(" user ", " pass:word ").await;
    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(2))
        .build()
        .unwrap();
    let unauthenticated = client.get(format!("{url}/status")).send().await.unwrap();
    assert_eq!(unauthenticated.status(), StatusCode::UNAUTHORIZED);
    assert!(
        unauthenticated
            .headers()
            .get(header::WWW_AUTHENTICATE)
            .is_none()
    );
    let bootstrap = client
        .get(format!("{url}/auth/session"))
        .send()
        .await
        .unwrap();
    assert_eq!(bootstrap.status(), StatusCode::UNAUTHORIZED);
    assert!(bootstrap.headers().get(header::WWW_AUTHENTICATE).is_none());
    let login = client
        .post(format!("{url}/auth/login"))
        .header(header::AUTHORIZATION, basic(" user : pass:word "))
        .json(&json!({"username":"unrelated","password":"ignored"}))
        .send()
        .await
        .unwrap();
    assert_eq!(login.status(), StatusCode::OK);
    assert!(login.headers().get(header::SET_COOKIE).is_none());
    assert_eq!(
        login.json::<Value>().await.unwrap(),
        json!({"authenticated":true,"username":" user "})
    );
    for invalid in [
        "Basic !!!".to_string(),
        "Digest anything".to_string(),
        basic("user: pass:word "),
        basic(" user :wrong"),
    ] {
        let response = client
            .get(format!("{url}/status"))
            .header(header::AUTHORIZATION, invalid)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }
    task.abort();
    let _ = task.await;
}

#[tokio::test]
async fn empty_username_disables_authentication_as_in_original_webui() {
    let (url, task) = server("", "unused-password").await;
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    assert_eq!(
        client
            .get(format!("{url}/status"))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    let session = client
        .get(format!("{url}/auth/session"))
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    assert_eq!(session, json!({"authenticated":true,"username":""}));
    task.abort();
    let _ = task.await;
}

#[tokio::test]
async fn logout_is_client_side_and_credentials_are_required_on_each_protected_request() {
    let (url, task) = server("webuser", "webpass").await;
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    let login = client
        .post(format!("{url}/auth/login"))
        .header(header::AUTHORIZATION, basic("webuser:webpass"))
        .send()
        .await
        .unwrap();
    assert_eq!(login.status(), StatusCode::OK);
    assert!(login.headers().get(header::SET_COOKIE).is_none());
    let logout = client
        .post(format!("{url}/auth/logout"))
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    assert_eq!(logout, json!({"authenticated":false}));
    assert_eq!(
        client
            .get(format!("{url}/status"))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        client
            .get(format!("{url}/status"))
            .header(header::AUTHORIZATION, basic("webuser:webpass"))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    task.abort();
    let _ = task.await;
}

#[tokio::test]
async fn full_length_difference_and_empty_password_follow_original_comparison_rules() {
    let state = AuthState::new(WebUiAuthConfig {
        user: format!("user{}", "\0".repeat(256)),
        pass: String::new(),
        ..Default::default()
    })
    .await;
    let mut headers = axum::http::HeaderMap::new();
    headers.insert(header::AUTHORIZATION, basic("user").parse().unwrap());
    assert!(!state.authenticated(&headers));
    let empty_pass = AuthState::new(WebUiAuthConfig {
        user: "user".into(),
        pass: String::new(),
        ..Default::default()
    })
    .await;
    assert!(empty_pass.authenticated(&headers));
    headers.insert(header::AUTHORIZATION, basic("user:bad").parse().unwrap());
    assert!(!empty_pass.authenticated(&headers));
}

#[tokio::test]
async fn remembered_sessions_persist_without_passwords_and_keep_cookie_authentication() {
    let directory = tempfile::tempdir().unwrap();
    let settings = WebUiAuthConfig {
        user: "session-user".into(),
        pass: "session-password".into(),
        ..Default::default()
    };
    let state = AuthState::with_state_dir(settings.clone(), directory.path())
        .await
        .unwrap();
    let (url, task) = server_with_state(state).await;
    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(2))
        .build()
        .unwrap();
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    for (remember, lifetime) in [(false, 604800u64), (true, 2592000u64)] {
        let login = client
            .post(format!("{url}/auth/login"))
            .header(
                header::AUTHORIZATION,
                basic("session-user:session-password"),
            )
            .json(&json!({"remember":remember}))
            .send()
            .await
            .unwrap();
        assert_eq!(login.status(), StatusCode::OK);
        assert!(login.headers().get(header::WWW_AUTHENTICATE).is_none());
        let cookie = login.headers()[header::SET_COOKIE]
            .to_str()
            .unwrap()
            .to_string();
        assert!(cookie.contains("HttpOnly") && cookie.contains("SameSite=Strict"));
        assert_eq!(cookie.contains("Max-Age="), remember);
        if remember {
            assert!(cookie.contains("Max-Age=2592000"));
        }
        assert!(!cookie.contains("session-password"));
        let cookie_pair = cookie.split(';').next().unwrap();
        let token = cookie_pair.split_once('=').unwrap().1;
        let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(token)
            .unwrap();
        let expires = u64::from_be_bytes(payload[1..9].try_into().unwrap());
        assert!(expires >= now + lifetime && expires <= now + lifetime + 5);
        assert!(!String::from_utf8_lossy(&payload).contains("session-password"));
        let persisted = AuthState::with_state_dir(settings.clone(), directory.path())
            .await
            .unwrap();
        let mut headers = axum::http::HeaderMap::new();
        headers.insert(header::COOKIE, cookie_pair.parse().unwrap());
        assert!(persisted.authenticated(&headers));
        let response = client
            .get(format!("{url}/status"))
            .header(header::COOKIE, cookie_pair)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let csrf = client
            .post(format!("{url}/status"))
            .header(header::COOKIE, cookie_pair)
            .header(header::ORIGIN, "https://other.example")
            .send()
            .await
            .unwrap();
        assert_eq!(csrf.status(), StatusCode::FORBIDDEN);
        let rejected = client
            .get(format!("{url}/status"))
            .header(header::COOKIE, "ikuai_failover_session=expired-or-invalid")
            .send()
            .await
            .unwrap();
        assert_eq!(rejected.status(), StatusCode::UNAUTHORIZED);
        assert!(rejected.headers().get(header::WWW_AUTHENTICATE).is_none());
        assert_eq!(
            rejected.json::<Value>().await.unwrap(),
            json!({"authenticated":false,"error":"Unauthorized"})
        );
    }
    task.abort();
    let _ = task.await;
}

#[test]
fn session_configuration_accepts_existing_accounts_and_rejects_invalid_lifetimes() {
    let existing: WebUiAuthConfig = serde_json::from_value(json!({"user":"u","pass":"p"})).unwrap();
    assert_eq!(existing.session_lifetime_seconds, 604800);
    assert_eq!(existing.remember_lifetime_seconds, 2592000);
    assert!(existing.validate().is_ok());
    let mut invalid = existing;
    invalid.session_lifetime_seconds = 0;
    assert!(invalid.validate().is_err());
    invalid.session_lifetime_seconds = 2592001;
    assert!(invalid.validate().is_err());
}
