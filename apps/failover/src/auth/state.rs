use std::{
    io::ErrorKind,
    path::Path,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use axum::http::{HeaderMap, header};
use base64::Engine;
use hmac::{Hmac, Mac};
use serde::Deserialize;
use sha2::Sha256;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

pub(crate) const SESSION_COOKIE: &str = "ikuai_failover_session";
const SESSION_LIFETIME: u64 = 7 * 24 * 60 * 60;
const REMEMBER_LIFETIME: u64 = 30 * 24 * 60 * 60;
const PAYLOAD_LENGTH: usize = 1 + 8 + 16;
const SIGNATURE_LENGTH: usize = 32;

#[derive(Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct WebUiAuthConfig {
    pub user: String,
    pub pass: String,
    pub session_lifetime_seconds: u64,
    pub remember_lifetime_seconds: u64,
}

impl Default for WebUiAuthConfig {
    fn default() -> Self {
        Self {
            user: String::new(),
            pass: String::new(),
            session_lifetime_seconds: SESSION_LIFETIME,
            remember_lifetime_seconds: REMEMBER_LIFETIME,
        }
    }
}

impl WebUiAuthConfig {
    pub fn validate(&self) -> Result<(), String> {
        if !(60..=365 * 24 * 60 * 60).contains(&self.session_lifetime_seconds)
            || !(60..=365 * 24 * 60 * 60).contains(&self.remember_lifetime_seconds)
            || self.remember_lifetime_seconds < self.session_lifetime_seconds
        {
            return Err("Session lifetimes must be between 60 seconds and 365 days, and remembered sessions must not be shorter than ordinary sessions".into());
        }
        Ok(())
    }

    pub fn apply_environment(&mut self) {
        for (key, field) in [
            ("IKB_WEBUI_USER", &mut self.user),
            ("IKB_WEBUI_PASS", &mut self.pass),
        ] {
            if let Some(value) = crate::config::optional_secret(&Some(key.to_string())) {
                if !value.trim().is_empty() {
                    *field = value.trim().to_string();
                }
            }
        }
    }
}

pub struct AuthState {
    settings: WebUiAuthConfig,
    session_key: Option<[u8; SIGNATURE_LENGTH]>,
}

impl AuthState {
    pub async fn new(settings: WebUiAuthConfig) -> Arc<Self> {
        Arc::new(Self {
            settings,
            session_key: None,
        })
    }

    pub async fn with_state_dir(
        settings: WebUiAuthConfig,
        state_dir: &Path,
    ) -> Result<Arc<Self>, String> {
        settings.validate()?;
        tokio::fs::create_dir_all(state_dir)
            .await
            .map_err(|_| "The console session directory could not be created".to_string())?;
        let session_key = load_session_key(state_dir).await?;
        Ok(Arc::new(Self {
            settings,
            session_key: Some(session_key),
        }))
    }

    pub fn username(&self) -> &str {
        &self.settings.user
    }

    pub fn authenticated(&self, headers: &HeaderMap) -> bool {
        self.basic_authenticated(headers) || self.cookie_authenticated(headers)
    }

    pub(crate) fn basic_authenticated(&self, headers: &HeaderMap) -> bool {
        if self.settings.user.is_empty() {
            return true;
        }
        let authorization = headers
            .get(header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default();
        let Some(encoded) = authorization.strip_prefix("Basic ") else {
            return false;
        };
        let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(encoded.as_bytes()) else {
            return false;
        };
        let Ok(credentials) = String::from_utf8(bytes) else {
            return false;
        };
        let mut parts = credentials.splitn(2, ':');
        let username = parts.next().unwrap_or_default();
        let password = parts.next().unwrap_or_default();
        constant_time_eq(username.as_bytes(), self.settings.user.as_bytes())
            && constant_time_eq(password.as_bytes(), self.settings.pass.as_bytes())
    }

    pub(crate) fn cookie_authenticated(&self, headers: &HeaderMap) -> bool {
        if self.session_key.is_none() || self.settings.user.is_empty() {
            return false;
        }
        headers.get_all(header::COOKIE).iter().any(|value| {
            value.to_str().is_ok_and(|cookies| {
                cookies.split(';').any(|cookie| {
                    let Some((name, token)) = cookie.trim().split_once('=') else {
                        return false;
                    };
                    name == SESSION_COOKIE && self.valid_token(token)
                })
            })
        })
    }

    pub(crate) fn session_lifetime(&self, remember: bool) -> u64 {
        if remember {
            self.settings.remember_lifetime_seconds
        } else {
            self.settings.session_lifetime_seconds
        }
    }

    pub(crate) fn issue_token(&self, remember: bool) -> Result<Option<String>, String> {
        if self.session_key.is_none() || self.settings.user.is_empty() {
            return Ok(None);
        }
        let expires = now_seconds()
            .and_then(|now| now.checked_add(self.session_lifetime(remember)))
            .ok_or_else(|| "The console session expiry could not be determined".to_string())?;
        let mut payload = [0u8; PAYLOAD_LENGTH];
        payload[0] = 1;
        payload[1..9].copy_from_slice(&expires.to_be_bytes());
        getrandom::fill(&mut payload[9..])
            .map_err(|_| "The console session could not be generated".to_string())?;
        let mac = self
            .token_mac(&payload)
            .ok_or_else(|| "The console session could not be signed".to_string())?;
        let signature = mac.finalize().into_bytes();
        let mut token = Vec::with_capacity(PAYLOAD_LENGTH + SIGNATURE_LENGTH);
        token.extend_from_slice(&payload);
        token.extend_from_slice(&signature);
        Ok(Some(
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(token),
        ))
    }

    fn token_mac(&self, payload: &[u8]) -> Option<Hmac<Sha256>> {
        let key = self.session_key.as_ref()?;
        let mut mac = Hmac::<Sha256>::new_from_slice(key).ok()?;
        mac.update(b"ikuai-failover.console-session.v1");
        mac.update(payload);
        // Changing either configured credential invalidates sessions signed with the old value.
        for credential in [&self.settings.user, &self.settings.pass] {
            mac.update(&(credential.len() as u64).to_be_bytes());
            mac.update(credential.as_bytes());
        }
        Some(mac)
    }

    fn valid_token(&self, token: &str) -> bool {
        if token.len() > 128 {
            return false;
        }
        let Ok(bytes) = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(token) else {
            return false;
        };
        if bytes.len() != PAYLOAD_LENGTH + SIGNATURE_LENGTH || bytes[0] != 1 {
            return false;
        }
        let Ok(expiry_bytes) = <[u8; 8]>::try_from(&bytes[1..9]) else {
            return false;
        };
        let Some(now) = now_seconds() else {
            return false;
        };
        if u64::from_be_bytes(expiry_bytes) <= now {
            return false;
        }
        self.token_mac(&bytes[..PAYLOAD_LENGTH])
            .is_some_and(|mac| mac.verify_slice(&bytes[PAYLOAD_LENGTH..]).is_ok())
    }
}

fn now_seconds() -> Option<u64> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(|duration| duration.as_secs())
}

async fn read_session_key(path: &Path) -> Result<[u8; SIGNATURE_LENGTH], std::io::Error> {
    let file = tokio::fs::File::open(path).await?;
    let mut bytes = Vec::with_capacity(SIGNATURE_LENGTH + 1);
    file.take((SIGNATURE_LENGTH + 1) as u64)
        .read_to_end(&mut bytes)
        .await?;
    bytes.try_into().map_err(|_| {
        std::io::Error::new(ErrorKind::InvalidData, "Invalid console session key length")
    })
}

async fn load_session_key(state_dir: &Path) -> Result<[u8; SIGNATURE_LENGTH], String> {
    let key_path = state_dir.join("auth-session.key");
    match read_session_key(&key_path).await {
        Ok(key) => return Ok(key),
        Err(error) if error.kind() == ErrorKind::NotFound => {}
        Err(_) => return Err("The stored console session key could not be read".into()),
    }
    let mut key = [0u8; SIGNATURE_LENGTH];
    let mut suffix = [0u8; 16];
    getrandom::fill(&mut key)
        .and_then(|_| getrandom::fill(&mut suffix))
        .map_err(|_| "The console session key could not be generated".to_string())?;
    let temporary = state_dir.join(format!(
        ".auth-session-{}.tmp",
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(suffix)
    ));
    let mut options = tokio::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    options.mode(0o600);
    let mut file = options
        .open(&temporary)
        .await
        .map_err(|_| "The console session key could not be created".to_string())?;
    let prepared = async {
        file.write_all(&key).await?;
        file.sync_all().await
    }
    .await;
    drop(file);
    if prepared.is_err() {
        let _ = tokio::fs::remove_file(&temporary).await;
        return Err("The console session key could not be saved".into());
    }
    let published = tokio::fs::hard_link(&temporary, &key_path).await;
    let _ = tokio::fs::remove_file(&temporary).await;
    match published {
        Ok(()) => Ok(key),
        Err(error) if error.kind() == ErrorKind::AlreadyExists => read_session_key(&key_path)
            .await
            .map_err(|_| "The stored console session key could not be read".to_string()),
        Err(_) => Err("The console session key could not be saved".into()),
    }
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    let maximum = left.len().max(right.len());
    let mut difference: usize = left.len() ^ right.len();
    for index in 0..maximum {
        difference |= usize::from(
            left.get(index).copied().unwrap_or(0) ^ right.get(index).copied().unwrap_or(0),
        );
    }
    difference == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expired_signed_sessions_are_rejected_and_existing_v1_sessions_still_work() {
        let state = AuthState {
            settings: WebUiAuthConfig {
                user: "u".into(),
                pass: "p".into(),
                ..Default::default()
            },
            session_key: Some([7; SIGNATURE_LENGTH]),
        };
        let mut payload = [0u8; PAYLOAD_LENGTH];
        payload[0] = 1;
        for (expires, valid) in [
            (now_seconds().unwrap() - 1, false),
            (now_seconds().unwrap() + 604800, true),
        ] {
            payload[1..9].copy_from_slice(&expires.to_be_bytes());
            let mac = state.token_mac(&payload).unwrap();
            let mut bytes = payload.to_vec();
            bytes.extend_from_slice(&mac.finalize().into_bytes());
            let token = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes);
            assert_eq!(state.valid_token(&token), valid);
        }
    }
}
