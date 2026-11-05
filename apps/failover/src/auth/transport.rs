use std::sync::Arc;

use axum::{
    Json, Router,
    body::{Body, Bytes},
    extract::{DefaultBodyLimit, State},
    http::{HeaderMap, Method, Request, StatusCode, Uri, header, uri::Authority},
    middleware::Next,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde::Deserialize;
use serde_json::json;

use super::AuthState;
use super::state::SESSION_COOKIE;

#[derive(Default, Deserialize)]
#[serde(default)]
struct LoginOptions {
    remember: bool,
}

pub fn routes(state: Arc<AuthState>) -> Router {
    Router::new()
        .route("/auth/session", get(session))
        .route(
            "/auth/login",
            post(login).layer(DefaultBodyLimit::max(1024)),
        )
        .route("/auth/logout", post(logout))
        .with_state(state)
}

async fn session(State(state): State<Arc<AuthState>>, headers: HeaderMap) -> Response {
    if !state.authenticated(&headers) {
        return unauthorized();
    }
    no_store(Json(json!({"authenticated":true,"username":state.username()})).into_response())
}

async fn login(
    State(state): State<Arc<AuthState>>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Response {
    if cross_origin(&headers) {
        return forbidden();
    }
    // Only the submitted account/password pair can create a new persistent session.
    if !state.basic_authenticated(&headers) {
        return unauthorized();
    }
    let options = if body.is_empty() {
        LoginOptions::default()
    } else {
        match serde_json::from_slice::<LoginOptions>(&body) {
            Ok(options) => options,
            Err(_) => {
                return no_store(
                    (
                        StatusCode::BAD_REQUEST,
                        Json(json!({"authenticated":false,"error":"Invalid login options"})),
                    )
                        .into_response(),
                );
            }
        }
    };
    let token = match state.issue_token(options.remember) {
        Ok(token) => token,
        Err(_) => {
            return no_store(
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({"authenticated":false,"error":"Session unavailable"})),
                )
                    .into_response(),
            );
        }
    };
    let mut response =
        Json(json!({"authenticated":true,"username":state.username()})).into_response();
    if let Some(token) = token {
        let secure = if uri.scheme_str() == Some("https") {
            "; Secure"
        } else {
            ""
        };
        let persistence = if options.remember {
            format!("; Max-Age={}", state.session_lifetime(true))
        } else {
            String::new()
        };
        let cookie = format!(
            "{SESSION_COOKIE}={token}; Path=/; HttpOnly; SameSite=Strict{persistence}{secure}"
        );
        let Ok(value) = cookie.parse() else {
            return no_store(
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({"authenticated":false,"error":"Session unavailable"})),
                )
                    .into_response(),
            );
        };
        response.headers_mut().insert(header::SET_COOKIE, value);
    }
    no_store(response)
}

async fn logout(headers: HeaderMap) -> Response {
    if cross_origin(&headers) {
        return forbidden();
    }
    let mut response = Json(json!({"authenticated":false})).into_response();
    response.headers_mut().insert(
        header::SET_COOKIE,
        axum::http::HeaderValue::from_static(
            "ikuai_failover_session=; Path=/; HttpOnly; SameSite=Strict; Max-Age=0",
        ),
    );
    no_store(response)
}

pub async fn protect(
    State(state): State<Arc<AuthState>>,
    request: Request<Body>,
    next: Next,
) -> Response {
    let basic = state.basic_authenticated(request.headers());
    let cookie = state.cookie_authenticated(request.headers());
    if basic || cookie {
        if !basic && !safe_method(request.method()) && cross_origin(request.headers()) {
            return forbidden();
        }
        return next.run(request).await;
    }
    unauthorized()
}

fn safe_method(method: &Method) -> bool {
    matches!(*method, Method::GET | Method::HEAD | Method::OPTIONS)
}

fn cross_origin(headers: &HeaderMap) -> bool {
    if headers
        .get("sec-fetch-site")
        .and_then(|value| value.to_str().ok())
        .is_some_and(|site| site.eq_ignore_ascii_case("cross-site"))
    {
        return true;
    }
    let Some(origin) = headers.get(header::ORIGIN) else {
        return false;
    };
    let (Ok(origin), Some(host)) = (origin.to_str(), headers.get(header::HOST)) else {
        return true;
    };
    let (Ok(origin), Ok(host)) = (reqwest::Url::parse(origin), host.to_str()) else {
        return true;
    };
    let Ok(authority) = host.parse::<Authority>() else {
        return true;
    };
    if !matches!(origin.scheme(), "http" | "https")
        || origin.path() != "/"
        || origin.query().is_some()
        || origin.fragment().is_some()
        || !origin.username().is_empty()
        || origin.password().is_some()
    {
        return true;
    }
    let origin_host = origin
        .host_str()
        .unwrap_or_default()
        .trim_matches(['[', ']']);
    let request_host = authority.host().trim_matches(['[', ']']);
    let request_port = authority
        .port_u16()
        .unwrap_or(if origin.scheme() == "https" { 443 } else { 80 });
    !origin_host.eq_ignore_ascii_case(request_host)
        || origin.port_or_known_default() != Some(request_port)
}

fn forbidden() -> Response {
    no_store(
        (
            StatusCode::FORBIDDEN,
            Json(json!({"authenticated":false,"error":"Cross-origin request rejected"})),
        )
            .into_response(),
    )
}

fn unauthorized() -> Response {
    let response = (
        StatusCode::UNAUTHORIZED,
        Json(json!({"authenticated":false,"error":"Unauthorized"})),
    )
        .into_response();
    no_store(response)
}

fn no_store(mut response: Response) -> Response {
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        axum::http::HeaderValue::from_static("no-store"),
    );
    response
}
