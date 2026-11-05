use std::{
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU8, Ordering},
    },
    time::Duration,
};

use axum::{
    Json,
    extract::{State, rejection::JsonRejection},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::io::AsyncWriteExt;

use crate::{runtime::Runtime, state::Egress};

#[derive(Clone, Copy, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SelectionMode {
    #[default]
    Auto,
    Primary,
    Backup,
}

impl SelectionMode {
    pub fn target(self) -> Option<Egress> {
        match self {
            Self::Auto => None,
            Self::Primary => Some(Egress::Primary),
            Self::Backup => Some(Egress::Backup),
        }
    }

    fn code(self) -> u8 {
        match self {
            Self::Auto => 0,
            Self::Primary => 1,
            Self::Backup => 2,
        }
    }
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct StoredSelection {
    version: u8,
    mode: SelectionMode,
}

pub(crate) struct SelectionStore {
    path: PathBuf,
    mode: AtomicU8,
    restart_resets: AtomicBool,
}

impl SelectionStore {
    pub async fn open(directory: &Path) -> Result<Self, String> {
        let path = directory.join("wan-selection.json");
        let mode = match tokio::fs::read(&path).await {
            Ok(bytes) => {
                if bytes.len() > 4096 {
                    return Err("Saved WAN selection is too large".into());
                }
                let saved: StoredSelection = serde_json::from_slice(&bytes)
                    .map_err(|_| "Saved WAN selection is invalid".to_string())?;
                if saved.version != 1 {
                    return Err("Saved WAN selection version is unsupported".into());
                }
                saved.mode
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => SelectionMode::Auto,
            Err(_) => return Err("Saved WAN selection cannot be read".into()),
        };
        Ok(Self {
            path,
            mode: AtomicU8::new(mode.code()),
            restart_resets: AtomicBool::new(false),
        })
    }

    pub fn mode(&self) -> SelectionMode {
        match self.mode.load(Ordering::SeqCst) {
            1 => SelectionMode::Primary,
            2 => SelectionMode::Backup,
            _ => SelectionMode::Auto,
        }
    }

    pub fn restart_resets(&self) -> bool {
        self.restart_resets.load(Ordering::SeqCst)
    }

    pub fn restore_in_memory(&self, mode: SelectionMode) {
        self.mode.store(mode.code(), Ordering::SeqCst);
        self.restart_resets.store(true, Ordering::SeqCst);
    }

    pub async fn save(&self, mode: SelectionMode) -> Result<(), String> {
        let bytes = serde_json::to_vec(&StoredSelection { version: 1, mode })
            .map_err(|_| "WAN selection cannot be encoded".to_string())?;
        let mut random = [0u8; 16];
        getrandom::fill(&mut random)
            .map_err(|_| "WAN selection temporary name cannot be generated".to_string())?;
        let temporary = self.path.with_file_name(format!(
            ".wan-selection-{}.tmp",
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(random)
        ));
        let mut options = tokio::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        options.mode(0o600);
        let mut file = options
            .open(&temporary)
            .await
            .map_err(|_| "WAN selection cannot be saved".to_string())?;
        let prepared = async {
            file.write_all(&bytes).await?;
            file.sync_all().await
        }
        .await;
        drop(file);
        if prepared.is_err() {
            let _ = tokio::fs::remove_file(&temporary).await;
            return Err("WAN selection write failed".into());
        }
        if tokio::fs::rename(&temporary, &self.path).await.is_err() {
            let _ = tokio::fs::remove_file(&temporary).await;
            return Err("WAN selection replacement failed".into());
        }
        self.mode.store(mode.code(), Ordering::SeqCst);
        self.restart_resets.store(false, Ordering::SeqCst);
        Ok(())
    }
}

pub(crate) struct SelectionError {
    pub status: StatusCode,
    pub code: &'static str,
    pub message: &'static str,
    pub blockers: Vec<String>,
}

impl SelectionError {
    pub fn new(status: StatusCode, code: &'static str, message: &'static str) -> Self {
        Self {
            status,
            code,
            message,
            blockers: Vec::new(),
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SelectionRequest {
    mode: SelectionMode,
}

pub async fn get(State(runtime): State<Arc<Runtime>>) -> Response {
    no_store(Json(runtime.wan_selection().await).into_response())
}

pub async fn set(
    State(runtime): State<Arc<Runtime>>,
    request: Result<Json<SelectionRequest>, JsonRejection>,
) -> Response {
    let result = match request {
        Ok(Json(request)) => {
            match tokio::time::timeout(Duration::from_secs(120), runtime.select_wan(request.mode))
                .await
            {
                Ok(result) => result,
                Err(_) => Err(SelectionError::new(
                    StatusCode::SERVICE_UNAVAILABLE,
                    "wan_selection_timeout",
                    "切换请求超时，请刷新线路状态确认当前模式，服务会继续按已保存的模式协调。",
                )),
            }
        }
        Err(_) => Err(SelectionError::new(
            StatusCode::BAD_REQUEST,
            "invalid_wan_selection",
            "请选择自动、主线路或备用线路。",
        )),
    };
    let mut selection = runtime.wan_selection().await;
    match result {
        Ok(()) => { selection["success"] = json!(true); no_store(Json(selection).into_response()) }
        Err(error) => no_store((error.status, Json(json!({"success":false,"error":error.code,"message":error.message,"blockers":error.blockers,"selection":selection}))).into_response()),
    }
}

fn no_store(mut response: Response) -> Response {
    response.headers_mut().insert(
        axum::http::header::CACHE_CONTROL,
        axum::http::HeaderValue::from_static("no-store"),
    );
    response
}
