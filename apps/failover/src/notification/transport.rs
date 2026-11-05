use std::{collections::HashMap, time::Duration};

use reqwest::{Client, RequestBuilder, Response};
use serde_json::Value;
use thiserror::Error;

use super::config::ProviderConfig;

#[derive(Clone, Copy, Debug, Error)]
pub enum NotificationError {
    #[error("invalid_configuration")]
    InvalidConfiguration,
    #[error("secret_missing")]
    SecretMissing,
    #[error("network_error")]
    Network,
    #[error("http_rejected")]
    HttpRejected,
    #[error("provider_rejected")]
    ProviderRejected,
    #[error("invalid_response")]
    InvalidResponse,
    #[error("timeout")]
    Timeout,
    #[error("crypto_error")]
    Crypto,
}

pub(crate) struct PreparedChannel {
    pub provider: ProviderConfig,
    pub secrets: HashMap<String, String>,
    pub mp_config: std::collections::BTreeMap<String, Value>,
    pub message_format: String,
}

impl PreparedChannel {
    pub fn secret(&self, name: &str) -> Result<&str, NotificationError> {
        self.secrets
            .get(name)
            .map(String::as_str)
            .ok_or(NotificationError::SecretMissing)
    }

    pub fn optional_secret(
        &self,
        name: &Option<String>,
    ) -> Result<Option<&str>, NotificationError> {
        Ok(name
            .as_deref()
            .and_then(|name| self.secrets.get(name))
            .map(String::as_str))
    }
}

pub(crate) fn secret_env(name: &str) -> Result<String, NotificationError> {
    std::env::var(name)
        .ok()
        .filter(|value| !value.trim().is_empty())
        .ok_or(NotificationError::SecretMissing)
}

pub(crate) fn client(timeout_ms: u64) -> Result<Client, NotificationError> {
    Client::builder()
        .timeout(Duration::from_millis(timeout_ms))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| NotificationError::InvalidConfiguration)
}

pub(crate) async fn response(request: RequestBuilder) -> Result<Response, NotificationError> {
    let response = request.send().await.map_err(|error| {
        if error.is_timeout() {
            NotificationError::Timeout
        } else {
            NotificationError::Network
        }
    })?;
    if !response.status().is_success() {
        return Err(NotificationError::HttpRejected);
    }
    Ok(response)
}

pub(crate) async fn json_response(request: RequestBuilder) -> Result<Value, NotificationError> {
    let response = response(request).await?;
    decode_json(response).await
}

// Some providers return an actionable format rejection in a non-2xx JSON response.
pub(crate) async fn json_response_with_status(
    request: RequestBuilder,
) -> Result<(reqwest::StatusCode, Value), NotificationError> {
    let response = request.send().await.map_err(|error| {
        if error.is_timeout() {
            NotificationError::Timeout
        } else {
            NotificationError::Network
        }
    })?;
    let status = response.status();
    Ok((status, decode_json(response).await?))
}

async fn decode_json(response: Response) -> Result<Value, NotificationError> {
    if response
        .content_length()
        .is_some_and(|length| length > 1_048_576)
    {
        return Err(NotificationError::InvalidResponse);
    }
    let mut response = response;
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| NotificationError::Network)?
    {
        if bytes.len().saturating_add(chunk.len()) > 1_048_576 {
            return Err(NotificationError::InvalidResponse);
        }
        bytes.extend_from_slice(&chunk);
    }
    let value: Value =
        serde_json::from_slice(&bytes).map_err(|_| NotificationError::InvalidResponse)?;
    if !value.is_object() {
        return Err(NotificationError::InvalidResponse);
    }
    Ok(value)
}

pub(crate) fn accept(condition: bool) -> Result<(), NotificationError> {
    if condition {
        Ok(())
    } else {
        Err(NotificationError::ProviderRejected)
    }
}

pub(crate) fn endpoint(base: &str, path: &str) -> String {
    format!(
        "{}/{}",
        base.trim_end_matches('/'),
        path.trim_start_matches('/')
    )
}
