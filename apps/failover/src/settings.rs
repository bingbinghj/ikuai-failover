use std::{path::Path, sync::Arc};

use axum::{
    Json,
    extract::{State, rejection::JsonRejection},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use serde_yaml::{Mapping, Value};
use sha2::{Digest, Sha256};
use tokio::io::AsyncReadExt;

use crate::{
    config::{self, Config, IkuaiConfig},
    runtime::Runtime,
};

const MAX_CONFIG_BYTES: u64 = 2 * 1024 * 1024;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SaveConfigRequest {
    revision: String,
    yaml: String,
}

#[derive(Serialize)]
struct ConfigurationDocument {
    revision: String,
    yaml: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SaveRequest {
    revision: String,
    base_url: String,
    username: String,
    #[serde(default)]
    password: Option<String>,
    #[serde(default)]
    clear_password: bool,
    allow_invalid_certificate: bool,
    timeout_ms: u64,
}

#[derive(Serialize)]
struct EnvironmentOverrides {
    username: bool,
    password: bool,
}

#[derive(Serialize)]
struct Settings {
    base_url: String,
    username: String,
    allow_invalid_certificate: bool,
    timeout_ms: u64,
    password_configured: bool,
    revision: String,
    environment_overrides: EnvironmentOverrides,
}

pub async fn get(State(runtime): State<Arc<Runtime>>) -> Response {
    let _gate = runtime.settings_gate.lock().await;
    let path = &runtime.config.source_path;
    if path.as_os_str().is_empty() {
        return unavailable();
    }
    let Ok((bytes, document)) = read_document(path).await else {
        return read_failed();
    };
    let Ok(settings) = settings(&document, &bytes) else {
        return read_failed();
    };
    no_store(Json(settings).into_response())
}

pub async fn get_config(State(runtime): State<Arc<Runtime>>) -> Response {
    let _gate = runtime.settings_gate.lock().await;
    let path = &runtime.config.source_path;
    if path.as_os_str().is_empty() {
        return unavailable();
    }
    // Keep the source text readable even if an external edit introduced invalid YAML.
    let Ok(bytes) = read_bytes(path).await else {
        return read_failed();
    };
    let Ok(yaml) = String::from_utf8(bytes.clone()) else {
        return read_failed();
    };
    no_store(
        Json(ConfigurationDocument {
            revision: revision(&bytes),
            yaml,
        })
        .into_response(),
    )
}

pub async fn save_config(
    State(runtime): State<Arc<Runtime>>,
    request: Result<Json<SaveConfigRequest>, JsonRejection>,
) -> Response {
    let Ok(Json(request)) = request else {
        return invalid_configuration(None);
    };
    if !valid_revision(&request.revision)
        || request.yaml.is_empty()
        || request.yaml.len() as u64 > MAX_CONFIG_BYTES
    {
        return invalid_configuration(None);
    }
    let document: Value = match serde_yaml::from_str(&request.yaml) {
        Ok(document) => document,
        Err(error) => return invalid_configuration(error.location()),
    };
    // Validation never initializes credentials, applies environment overrides, or contacts a router.
    if config::validate_candidate(&document).is_err() {
        return invalid_configuration(None);
    }
    let _gate = runtime.settings_gate.lock().await;
    let path = &runtime.config.source_path;
    if path.as_os_str().is_empty() {
        return unavailable();
    }
    let Ok(bytes) = read_bytes(path).await else {
        return read_failed();
    };
    if revision(&bytes) != request.revision {
        return configuration_changed();
    }
    match config::replace_document(path, &bytes, &request.yaml).await {
        Err(error) if error == config::CONFIG_FILE_CHANGED => return configuration_changed(),
        Err(_) => {
            return error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "configuration_save_failed",
                "Configuration could not be saved; ensure its directory is writable",
            );
        }
        Ok(()) => {}
    }
    let Ok(saved_bytes) = read_bytes(path).await else {
        return error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "configuration_readback_failed",
            "Configuration was saved but could not be read back; reload settings before retrying",
        );
    };
    if saved_bytes != request.yaml.as_bytes() {
        return configuration_changed();
    }
    if bytes != saved_bytes {
        runtime
            .notify(
                "configuration_saved",
                "服务配置已保存",
                "配置已保存，重启本服务后生效。",
            )
            .await;
    }
    no_store(
        Json(json!({
            "success": true,
            "restart_required": true,
            "revision": revision(&saved_bytes),
        }))
        .into_response(),
    )
}

pub async fn restart(State(runtime): State<Arc<Runtime>>) -> Response {
    runtime.request_restart();
    no_store(
        (
            StatusCode::ACCEPTED,
            Json(json!({ "success": true, "restarting": true })),
        )
            .into_response(),
    )
}

pub async fn save(
    State(runtime): State<Arc<Runtime>>,
    request: Result<Json<SaveRequest>, JsonRejection>,
) -> Response {
    let Ok(Json(request)) = request else {
        return invalid_request();
    };
    if !request.valid() {
        return invalid_request();
    }
    let _gate = runtime.settings_gate.lock().await;
    let path = &runtime.config.source_path;
    if path.as_os_str().is_empty() {
        return unavailable();
    }
    let Ok((bytes, document)) = read_document(path).await else {
        return read_failed();
    };
    if revision(&bytes) != request.revision {
        return configuration_changed();
    }
    // Reject an already invalid document without rewriting unrelated sections.
    if serde_yaml::from_value::<Config>(document).is_err() {
        return read_failed();
    }
    let saved = config::update_document(path, Some(&bytes), move |document| {
        let original = ikuai(document)?;
        let root = document
            .as_mapping_mut()
            .ok_or_else(|| "The configuration root is invalid".to_string())?;
        let section = root
            .get_mut(Value::String("ikuai".into()))
            .and_then(Value::as_mapping_mut)
            .ok_or_else(|| "The iKuai configuration section is invalid".to_string())?;
        insert_string(section, "base_url", request.base_url.trim().into());
        insert_string(section, "username", request.username);
        let password = if request.clear_password {
            String::new()
        } else {
            request
                .password
                .filter(|password| !password.is_empty())
                .unwrap_or(original.password)
        };
        insert_string(section, "password", password);
        section.insert(
            Value::String("allow_invalid_certificate".into()),
            Value::Bool(request.allow_invalid_certificate),
        );
        section.insert(
            Value::String("timeout_ms".into()),
            Value::Number(request.timeout_ms.into()),
        );
        Ok(())
    })
    .await;
    match saved {
        Err(error) if error == config::CONFIG_FILE_CHANGED => return configuration_changed(),
        Err(_) => {
            return error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "configuration_save_failed",
                "Configuration could not be saved; ensure its directory is writable",
            );
        }
        Ok(_) => {}
    }
    let Ok((saved_bytes, saved_document)) = read_document(path).await else {
        return error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "configuration_readback_failed",
            "Configuration was saved but could not be read back; reload settings before retrying",
        );
    };
    let Ok(mut saved_ikuai) = ikuai(&saved_document) else {
        return read_failed();
    };
    saved_ikuai.apply_environment();
    if bytes != saved_bytes {
        runtime
            .notify(
                "configuration_saved",
                "爱快连接配置已保存",
                "连接配置已保存，重启本服务后生效。",
            )
            .await;
    }
    no_store(
        Json(json!({
            "success": true,
            "restart_required": true,
            "configured": saved_ikuai.has_credentials(),
            "revision": revision(&saved_bytes),
        }))
        .into_response(),
    )
}

impl SaveRequest {
    fn valid(&self) -> bool {
        if !valid_revision(&self.revision)
            || self.base_url.len() > 2048
            || self.username.len() > 256
            || self.username.chars().any(char::is_control)
            || self.timeout_ms == 0
            || self.timeout_ms > 30000
            || self.password.as_ref().is_some_and(|password| {
                password.len() > 1024 || password.chars().any(char::is_control)
            })
            || (self.clear_password
                && self
                    .password
                    .as_ref()
                    .is_some_and(|password| !password.is_empty()))
        {
            return false;
        }
        reqwest::Url::parse(self.base_url.trim()).is_ok_and(|url| {
            url.scheme() == "https"
                && url.host_str().is_some()
                && url.username().is_empty()
                && url.password().is_none()
                && url.query().is_none()
                && url.fragment().is_none()
        })
    }
}

async fn read_document(path: &Path) -> Result<(Vec<u8>, Value), ()> {
    let bytes = read_bytes(path).await?;
    let document = serde_yaml::from_slice(&bytes).map_err(|_| ())?;
    Ok((bytes, document))
}

async fn read_bytes(path: &Path) -> Result<Vec<u8>, ()> {
    let file = tokio::fs::File::open(path).await.map_err(|_| ())?;
    if !file.metadata().await.map_err(|_| ())?.is_file() {
        return Err(());
    }
    let mut bytes = Vec::new();
    file.take(MAX_CONFIG_BYTES + 1)
        .read_to_end(&mut bytes)
        .await
        .map_err(|_| ())?;
    if bytes.len() as u64 > MAX_CONFIG_BYTES {
        return Err(());
    }
    Ok(bytes)
}

fn ikuai(document: &Value) -> Result<IkuaiConfig, String> {
    let value = document
        .get("ikuai")
        .cloned()
        .ok_or_else(|| "The iKuai configuration section is absent".to_string())?;
    serde_yaml::from_value(value).map_err(|_| "The iKuai configuration section is invalid".into())
}

fn settings(document: &Value, bytes: &[u8]) -> Result<Settings, String> {
    let stored = ikuai(document)?;
    Ok(Settings {
        base_url: stored.base_url,
        username: stored.username,
        allow_invalid_certificate: stored.allow_invalid_certificate,
        timeout_ms: stored.timeout_ms,
        password_configured: !stored.password.is_empty(),
        revision: revision(bytes),
        environment_overrides: EnvironmentOverrides {
            username: config::optional_secret(&Some("IKUAI_FAILOVER_USERNAME".into())).is_some(),
            password: config::optional_secret(&Some("IKUAI_FAILOVER_PASSWORD".into())).is_some(),
        },
    })
}

fn insert_string(mapping: &mut Mapping, key: &str, value: String) {
    mapping.insert(Value::String(key.into()), Value::String(value));
}

fn revision(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn valid_revision(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn invalid_configuration(location: Option<serde_yaml::Location>) -> Response {
    let mut body = json!({
        "success": false,
        "error": "invalid_configuration",
        "message": "Provide valid YAML with supported configuration fields and values",
    });
    if let Some(location) = location {
        body["line"] = json!(location.line());
        body["column"] = json!(location.column());
    }
    no_store((StatusCode::BAD_REQUEST, Json(body)).into_response())
}

fn invalid_request() -> Response {
    error(
        StatusCode::BAD_REQUEST,
        "invalid_settings",
        "Provide a valid HTTPS iKuai URL, revision, and timeout between 1 and 30000 milliseconds",
    )
}

fn configuration_changed() -> Response {
    error(
        StatusCode::CONFLICT,
        "configuration_changed",
        "Configuration changed since it was loaded; reload settings before saving",
    )
}

fn read_failed() -> Response {
    error(
        StatusCode::INTERNAL_SERVER_ERROR,
        "configuration_read_failed",
        "Configuration could not be read; check the mounted config.yml",
    )
}

fn unavailable() -> Response {
    error(
        StatusCode::SERVICE_UNAVAILABLE,
        "configuration_unavailable",
        "The configuration file is unavailable",
    )
}

fn error(status: StatusCode, code: &str, message: &str) -> Response {
    no_store(
        (
            status,
            Json(json!({ "success": false, "error": code, "message": message })),
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
