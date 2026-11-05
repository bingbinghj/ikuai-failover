use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::{self, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock},
    time::{SystemTime, UNIX_EPOCH},
};

use axum::{
    Json,
    extract::{Query, State, rejection::QueryRejection},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
};
use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::{config::Config, runtime::Runtime};

pub const MAX_FILE_BYTES: u64 = 5 * 1024 * 1024;
pub const MAX_FILES: usize = 10;
const MAX_MESSAGE_BYTES: usize = 4096;
const MAX_TAIL_LINES: usize = 1000;

static LOGGER: OnceLock<Mutex<Option<Arc<LogStore>>>> = OnceLock::new();

#[derive(Clone, Copy)]
enum Level {
    Debug,
    Info,
    Warn,
    Error,
}

impl Level {
    fn name(self) -> &'static str {
        match self {
            Self::Debug => "DEBUG",
            Self::Info => "INFO",
            Self::Warn => "WARN",
            Self::Error => "ERROR",
        }
    }
}

#[derive(Deserialize, Serialize)]
pub struct LogRecord {
    pub timestamp_ms: u64,
    pub level: String,
    pub message: String,
}

#[derive(Serialize)]
struct LogFile {
    name: String,
    size_bytes: u64,
    modified_ms: u64,
    active: bool,
}

struct Writer {
    file: Option<File>,
    size: u64,
    failure_reported: bool,
}

pub struct LogStore {
    directory: PathBuf,
    available: bool,
    writer: Mutex<Writer>,
    secrets: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TailQuery {
    file: Option<String>,
    lines: Option<usize>,
    level: Option<String>,
    search: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DownloadQuery {
    file: Option<String>,
}

#[derive(Serialize)]
struct Tail {
    file: String,
    lines: Vec<TailRecord>,
    truncated: bool,
}

#[derive(Serialize)]
struct TailRecord {
    #[serde(flatten)]
    record: LogRecord,
    file: String,
}

struct LogSnapshot {
    name: String,
    file: File,
    size: u64,
}

enum ReadError {
    InvalidName,
    Missing,
    Unavailable,
}

/// File logging is best-effort; its failure never stops WAN management.
pub fn initialize(config: &Config) -> Arc<LogStore> {
    let directory = config.management.state_dir.join("logs");
    let secrets = configuration_secrets(config);
    let opened = open_directory(&directory).and_then(|_| {
        trim_oversized_files(&directory)?;
        open_active(&directory)
    });
    let (available, writer) = match opened {
        Ok((file, size)) => (
            true,
            Writer {
                file: Some(file),
                size,
                failure_reported: false,
            },
        ),
        Err(_) => (
            false,
            Writer {
                file: None,
                size: 0,
                failure_reported: true,
            },
        ),
    };
    let store = Arc::new(LogStore {
        directory,
        available,
        writer: Mutex::new(writer),
        secrets,
    });
    if let Ok(mut logger) = LOGGER.get_or_init(|| Mutex::new(None)).lock() {
        *logger = Some(Arc::clone(&store));
    }
    if !available {
        console(&LogRecord {
            timestamp_ms: now_ms(),
            level: "WARN".into(),
            message: "日志文件不可用，继续输出到容器日志；请检查状态目录权限。".into(),
        });
    }
    store
}

pub fn debug(message: &str) {
    emit(Level::Debug, message);
}

pub fn info(message: &str) {
    emit(Level::Info, message);
}

pub fn warn(message: &str) {
    emit(Level::Warn, message);
}

pub fn error(message: &str) {
    emit(Level::Error, message);
}

pub fn flush() {
    if let Some(store) = logger() {
        store.flush();
    }
}

pub fn initialized() -> bool {
    logger().is_some()
}

fn logger() -> Option<Arc<LogStore>> {
    LOGGER
        .get()
        .and_then(|logger| logger.lock().ok())
        .and_then(|logger| logger.as_ref().map(Arc::clone))
}

fn emit(level: Level, message: &str) {
    if let Some(store) = logger() {
        store.record(level, message);
    } else {
        // Before configuration is loaded, emit only bounded diagnostics to stderr.
        let message = normalized_message(message);
        let mut stderr = io::stderr().lock();
        let _ = writeln!(stderr, "[{}] [{}] {message}", now_ms(), level.name());
    }
}

impl LogStore {
    fn record(&self, level: Level, message: &str) {
        let record = LogRecord {
            timestamp_ms: now_ms(),
            level: level.name().into(),
            message: self.redact(message),
        };
        console(&record);
        let Ok(mut bytes) = serde_json::to_vec(&record) else {
            return;
        };
        bytes.push(b'\n');
        let Ok(mut writer) = self.writer.lock() else {
            return;
        };
        if writer.file.is_none() {
            return;
        }
        let result = self.write(&mut writer, &bytes);
        if result.is_err() {
            writer.file = None;
            if !writer.failure_reported {
                writer.failure_reported = true;
                console(&LogRecord {
                    timestamp_ms: now_ms(),
                    level: "WARN".into(),
                    message: "日志文件写入失败，继续输出到容器日志；重启服务后重新尝试文件记录。"
                        .into(),
                });
            }
        }
    }

    fn write(&self, writer: &mut Writer, bytes: &[u8]) -> io::Result<()> {
        if bytes.len() as u64 > MAX_FILE_BYTES {
            return Err(io::Error::other("Log record exceeds its size limit"));
        }
        if writer.size.saturating_add(bytes.len() as u64) > MAX_FILE_BYTES {
            rotate(&self.directory, writer)?;
        }
        let file = writer
            .file
            .as_mut()
            .ok_or_else(|| io::Error::other("Log file is unavailable"))?;
        file.write_all(bytes)?;
        writer.size += bytes.len() as u64;
        Ok(())
    }

    fn flush(&self) {
        if let Ok(mut writer) = self.writer.lock() {
            if let Some(file) = writer.file.as_mut() {
                let _ = file.flush();
                let _ = file.sync_data();
            }
        }
    }

    fn redact(&self, message: &str) -> String {
        let mut value = message.to_string();
        // Long values first prevents a shorter credential obscuring its container.
        for secret in &self.secrets {
            value = value.replace(secret, "[已隐藏]");
        }
        normalized_message(&redact_urls(&value))
    }

    fn files(&self) -> Result<Vec<LogFile>, ReadError> {
        if !self.available {
            return Err(ReadError::Unavailable);
        }
        let _writer = self.writer.lock().map_err(|_| ReadError::Unavailable)?;
        self.listed_files()
    }

    fn listed_files(&self) -> Result<Vec<LogFile>, ReadError> {
        let mut files = Vec::new();
        for name in managed_file_names() {
            let path = self.directory.join(&name);
            match fs::symlink_metadata(&path) {
                Ok(metadata) => {
                    if !metadata.is_file() || metadata.len() > MAX_FILE_BYTES {
                        continue;
                    }
                    let active = name == file_name(0);
                    files.push(LogFile {
                        name,
                        size_bytes: metadata.len(),
                        modified_ms: metadata
                            .modified()
                            .ok()
                            .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
                            .map(milliseconds)
                            .unwrap_or_default(),
                        active,
                    });
                }
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(_) => return Err(ReadError::Unavailable),
            }
        }
        if !files.iter().any(|file| file.active) {
            if let Some(file) = files
                .iter_mut()
                .find(|file| file.name == server_file_name(0))
            {
                file.active = true;
            }
        }
        files.sort_by(|left, right| {
            right
                .active
                .cmp(&left.active)
                .then_with(|| right.modified_ms.cmp(&left.modified_ms))
                .then_with(|| left.name.cmp(&right.name))
        });
        files.truncate(MAX_FILES);
        Ok(files)
    }

    fn tail(&self, query: TailQuery) -> Result<Tail, ReadError> {
        if !self.available {
            return Err(ReadError::Unavailable);
        }
        let name = query.file.unwrap_or_else(|| file_name(0));
        let lines = query.lines.unwrap_or(300);
        let level = normalized_level(&query.level.unwrap_or_default());
        let search = query.search.unwrap_or_default();
        if lines == 0
            || lines > MAX_TAIL_LINES
            || !["", "ALL", "INFO", "WARN", "ERROR", "DEBUG"].contains(&level.as_str())
            || search.len() > 256
            || search.chars().any(char::is_control)
        {
            return Err(ReadError::InvalidName);
        }
        let search = search.to_lowercase();
        let mut entries = BTreeMap::new();
        let mut sequence = 0u64;
        let mut truncated = false;
        for snapshot in self.snapshots(&name)? {
            let source = snapshot.name.clone();
            let bytes = read_snapshot(snapshot)?;
            for line in bytes.split(|byte| *byte == b'\n') {
                let Ok(mut record) = serde_json::from_slice::<LogRecord>(line) else {
                    continue;
                };
                record.level = normalized_level(&record.level);
                if !["", "ALL"].contains(&level.as_str()) && record.level != level {
                    continue;
                }
                record.message = self.redact(&record.message);
                if !search.is_empty() {
                    let searchable = format!(
                        "{} {} {} {} {}",
                        source,
                        record.level,
                        if record.level == "WARN" {
                            "WARNING"
                        } else {
                            ""
                        },
                        timestamp(record.timestamp_ms),
                        record.message
                    )
                    .to_lowercase();
                    if !searchable.contains(&search) {
                        continue;
                    }
                }
                sequence = sequence.saturating_add(1);
                entries.insert(
                    (record.timestamp_ms, sequence),
                    TailRecord {
                        record,
                        file: source.clone(),
                    },
                );
                if entries.len() > lines {
                    entries.pop_first();
                    truncated = true;
                }
            }
        }
        Ok(Tail {
            file: name,
            lines: entries.into_values().collect(),
            truncated,
        })
    }

    fn download(&self, name: String) -> Result<(String, Vec<u8>), ReadError> {
        if !self.available {
            return Err(ReadError::Unavailable);
        }
        let mut bytes = Vec::new();
        for snapshot in self.snapshots(&name)? {
            let content = read_snapshot(snapshot)?;
            if name == "all" && !bytes.is_empty() && bytes.last() != Some(&b'\n') {
                bytes.push(b'\n');
            }
            bytes.extend_from_slice(&content);
            let maximum = if name == "all" {
                MAX_FILE_BYTES.saturating_mul(MAX_FILES as u64)
            } else {
                MAX_FILE_BYTES
            };
            if bytes.len() as u64 > maximum {
                return Err(ReadError::Unavailable);
            }
        }
        let filename = if name == "all" {
            "service-all.log".into()
        } else {
            name
        };
        Ok((filename, bytes))
    }

    fn snapshots(&self, name: &str) -> Result<Vec<LogSnapshot>, ReadError> {
        let all = name == "all";
        if !all && !valid_file_name(name) {
            return Err(ReadError::InvalidName);
        }
        let _writer = self.writer.lock().map_err(|_| ReadError::Unavailable)?;
        let names = if all {
            let mut files = self.listed_files()?;
            files.sort_by(|left, right| {
                left.active
                    .cmp(&right.active)
                    .then_with(|| left.modified_ms.cmp(&right.modified_ms))
                    .then_with(|| right.name.cmp(&left.name))
            });
            files.into_iter().map(|file| file.name).collect::<Vec<_>>()
        } else {
            vec![name.to_string()]
        };
        let mut snapshots = Vec::new();
        for name in names {
            let file = match checked_file(&self.directory, &name) {
                Ok(file) => file,
                Err(ReadError::Missing) if all => continue,
                Err(error) => return Err(error),
            };
            let size = file.metadata().map_err(|_| ReadError::Unavailable)?.len();
            if size > MAX_FILE_BYTES {
                return Err(ReadError::Unavailable);
            }
            snapshots.push(LogSnapshot { name, file, size });
        }
        Ok(snapshots)
    }
}

fn normalized_level(level: &str) -> String {
    let level = level.to_ascii_uppercase();
    if level == "WARNING" {
        "WARN".into()
    } else {
        level
    }
}

fn read_snapshot(snapshot: LogSnapshot) -> Result<Vec<u8>, ReadError> {
    let mut bytes = Vec::new();
    snapshot
        .file
        .take(snapshot.size)
        .read_to_end(&mut bytes)
        .map_err(|_| ReadError::Unavailable)?;
    if bytes.len() as u64 != snapshot.size {
        return Err(ReadError::Unavailable);
    }
    Ok(bytes)
}

fn open_directory(directory: &Path) -> io::Result<()> {
    fs::create_dir_all(directory)?;
    if !fs::symlink_metadata(directory)?.is_dir() {
        return Err(io::Error::other("Log directory is not a regular directory"));
    }
    for index in 0..MAX_FILES {
        regular_or_missing(&directory.join(file_name(index)))?;
    }
    Ok(())
}

fn regular_or_missing(path: &Path) -> io::Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_file() => Ok(true),
        Ok(_) => Err(io::Error::other("Managed log path is not a regular file")),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}

fn file_name(index: usize) -> String {
    if index == 0 {
        "service.log".into()
    } else {
        format!("service.{index}.log")
    }
}

fn valid_file_name(name: &str) -> bool {
    (0..MAX_FILES).any(|index| name == file_name(index) || name == server_file_name(index))
}

fn server_file_name(index: usize) -> String {
    if index == 0 {
        "server.log".into()
    } else {
        format!("server.log.{index}")
    }
}

fn managed_file_names() -> Vec<String> {
    (0..MAX_FILES)
        .flat_map(|index| [file_name(index), server_file_name(index)])
        .collect()
}

fn checked_file(directory: &Path, name: &str) -> Result<File, ReadError> {
    if !valid_file_name(name) {
        return Err(ReadError::InvalidName);
    }
    let path = directory.join(name);
    match regular_or_missing(&path) {
        Ok(false) => return Err(ReadError::Missing),
        Err(_) => return Err(ReadError::Unavailable),
        Ok(true) => {}
    }
    let root = fs::canonicalize(directory).map_err(|_| ReadError::Unavailable)?;
    let resolved = fs::canonicalize(&path).map_err(|_| ReadError::Unavailable)?;
    if resolved.parent() != Some(root.as_path()) {
        return Err(ReadError::Unavailable);
    }
    let file = File::open(&path).map_err(|_| ReadError::Unavailable)?;
    if !file
        .metadata()
        .map_err(|_| ReadError::Unavailable)?
        .is_file()
    {
        return Err(ReadError::Unavailable);
    }
    Ok(file)
}

fn open_active(directory: &Path) -> io::Result<(File, u64)> {
    let path = directory.join(file_name(0));
    regular_or_missing(&path)?;
    let file = OpenOptions::new().create(true).append(true).open(&path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.len() > MAX_FILE_BYTES {
        return Err(io::Error::other("Active log file is invalid"));
    }
    Ok((file, metadata.len()))
}

/// Imported oversized files retain only their newest complete records.
fn trim_oversized_files(directory: &Path) -> io::Result<()> {
    for index in 0..MAX_FILES {
        let path = directory.join(file_name(index));
        if !regular_or_missing(&path)? || fs::metadata(&path)?.len() <= MAX_FILE_BYTES {
            continue;
        }
        let mut file = OpenOptions::new().read(true).write(true).open(&path)?;
        let length = file.metadata()?.len();
        file.seek(SeekFrom::Start(length - MAX_FILE_BYTES))?;
        let mut bytes = Vec::new();
        Read::by_ref(&mut file)
            .take(MAX_FILE_BYTES)
            .read_to_end(&mut bytes)?;
        let first = bytes
            .iter()
            .position(|byte| *byte == b'\n')
            .map(|index| index + 1);
        let last = bytes.iter().rposition(|byte| *byte == b'\n');
        let retained = match (first, last) {
            (Some(first), Some(last)) if first <= last => &bytes[first..=last],
            _ => &[],
        };
        file.seek(SeekFrom::Start(0))?;
        file.write_all(retained)?;
        file.set_len(retained.len() as u64)?;
        file.sync_data()?;
    }
    Ok(())
}

fn rotate(directory: &Path, writer: &mut Writer) -> io::Result<()> {
    for index in 0..MAX_FILES {
        regular_or_missing(&directory.join(file_name(index)))?;
    }
    if let Some(mut file) = writer.file.take() {
        file.flush()?;
    }
    let oldest = directory.join(file_name(MAX_FILES - 1));
    if regular_or_missing(&oldest)? {
        fs::remove_file(oldest)?;
    }
    for index in (1..MAX_FILES - 1).rev() {
        let source = directory.join(file_name(index));
        if regular_or_missing(&source)? {
            fs::rename(source, directory.join(file_name(index + 1)))?;
        }
    }
    let active = directory.join(file_name(0));
    if regular_or_missing(&active)? {
        fs::rename(active, directory.join(file_name(1)))?;
    }
    let (file, size) = open_active(directory)?;
    writer.file = Some(file);
    writer.size = size;
    Ok(())
}

fn configuration_secrets(config: &Config) -> Vec<String> {
    let mut values = vec![config.ikuai.password.clone(), config.webui.pass.clone()];
    for (user, password) in [
        (&config.ikuai.username, &config.ikuai.password),
        (&config.webui.user, &config.webui.pass),
    ] {
        if !password.is_empty() {
            values.push(
                base64::engine::general_purpose::STANDARD.encode(format!("{user}:{password}")),
            );
        }
    }
    if !config.ikuai.password.is_empty() {
        values.push(format!("{:x}", md5::compute(&config.ikuai.password)));
        values.push(
            base64::engine::general_purpose::STANDARD
                .encode(format!("salt_11{}", config.ikuai.password)),
        );
    }
    for value in config.notifications.secrets.values() {
        values.push(value.clone());
        json_secrets(value, &mut values);
    }
    for channel in &config.notifications.channels {
        for (key, value) in &channel.mp_config {
            if [
                "SECRET",
                "TOKEN",
                "AESKEY",
                "API_KEY",
                "RELAY_KEY",
                "WEBHOOK",
            ]
            .iter()
            .any(|part| key.contains(part))
            {
                if let Some(value) = value.as_str().filter(|value| !value.is_empty()) {
                    values.push(value.to_string());
                }
            }
        }
        for key in channel
            .provider
            .required_env()
            .into_iter()
            .chain(channel.provider.optional_env())
        {
            if let Ok(value) = std::env::var(key) {
                json_secrets(&value, &mut values);
                values.push(value);
            }
        }
    }
    let encoded: Vec<String> = values
        .iter()
        .filter(|value| !value.is_empty())
        .flat_map(|value| {
            let json = serde_json::to_string(value)
                .ok()
                .map(|text| text[1..text.len() - 1].to_string());
            let mut escaped = String::new();
            for byte in value.as_bytes() {
                if byte.is_ascii_alphanumeric() || b"-._~".contains(byte) {
                    escaped.push(char::from(*byte));
                } else {
                    escaped.push_str(&format!("%{byte:02X}"));
                }
            }
            json.into_iter().chain(std::iter::once(escaped))
        })
        .collect();
    values.extend(encoded);
    values.retain(|value| !value.is_empty());
    values.sort_by(|a, b| b.len().cmp(&a.len()).then_with(|| a.cmp(b)));
    values.dedup();
    values
}

fn json_secrets(text: &str, output: &mut Vec<String>) {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(text) else {
        return;
    };
    let mut pending = vec![&value];
    let mut visited = 0;
    while let Some(value) = pending.pop() {
        visited += 1;
        if visited > 4096 {
            break;
        }
        match value {
            serde_json::Value::String(value) if !value.is_empty() => output.push(value.clone()),
            serde_json::Value::Array(items) => pending.extend(items),
            serde_json::Value::Object(items) => pending.extend(items.values()),
            _ => {}
        }
    }
}

fn redact_urls(message: &str) -> String {
    let mut result = String::new();
    let mut rest = message;
    while let Some(start) = rest
        .find("http://")
        .into_iter()
        .chain(rest.find("https://"))
        .min()
    {
        result.push_str(&rest[..start]);
        let suffix = &rest[start..];
        let end = suffix
            .find(|character: char| {
                character.is_whitespace() || matches!(character, '"' | '\'' | '<' | '>')
            })
            .unwrap_or(suffix.len());
        let url = &suffix[..end];
        let sensitive = reqwest::Url::parse(url).is_ok_and(|url| {
            !url.username().is_empty()
                || url.password().is_some()
                || url.query_pairs().any(|(key, _)| {
                    let key = key.to_ascii_lowercase();
                    [
                        "key",
                        "token",
                        "access_token",
                        "secret",
                        "password",
                        "passwd",
                        "signature",
                        "sign",
                    ]
                    .contains(&key.as_str())
                })
        });
        result.push_str(if sensitive {
            "[凭据地址已隐藏]"
        } else {
            url
        });
        rest = &suffix[end..];
    }
    result.push_str(rest);
    result
}

fn normalized_message(message: &str) -> String {
    let mut result = String::new();
    for character in message.chars() {
        let character = if character.is_control() {
            ' '
        } else {
            character
        };
        if result.len() + character.len_utf8() > MAX_MESSAGE_BYTES - "…".len() {
            result.push('…');
            break;
        }
        result.push(character);
    }
    result
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(milliseconds)
        .unwrap_or_default()
}

fn milliseconds(duration: std::time::Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

fn console(record: &LogRecord) {
    let mut output = io::stdout().lock();
    let _ = writeln!(
        output,
        "[{}] [{}] {}",
        timestamp(record.timestamp_ms),
        record.level,
        record.message
    );
}

pub(crate) fn timestamp(milliseconds: u64) -> String {
    let seconds = milliseconds / 1000;
    let days = (seconds / 86_400) as i64;
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36524 - day_of_era / 146096) / 365;
    let mut year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_part = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_part + 2) / 5 + 1;
    let month = month_part + if month_part < 10 { 3 } else { -9 };
    year += i64::from(month <= 2);
    let time = seconds % 86_400;
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{:03}Z",
        time / 3600,
        time / 60 % 60,
        time % 60,
        milliseconds % 1000
    )
}

pub async fn files(State(runtime): State<Arc<Runtime>>) -> Response {
    let logs = Arc::clone(&runtime.logs);
    match tokio::task::spawn_blocking(move || logs.files()).await {
        Ok(Ok(files)) => no_store(
            Json(json!({"files":files,"max_size_bytes":MAX_FILE_BYTES,"max_files":MAX_FILES}))
                .into_response(),
        ),
        Ok(Err(error)) => read_error(error),
        Err(_) => read_error(ReadError::Unavailable),
    }
}

pub async fn tail(
    State(runtime): State<Arc<Runtime>>,
    query: Result<Query<TailQuery>, QueryRejection>,
) -> Response {
    let Ok(Query(query)) = query else {
        return read_error(ReadError::InvalidName);
    };
    let logs = Arc::clone(&runtime.logs);
    match tokio::task::spawn_blocking(move || logs.tail(query)).await {
        Ok(Ok(tail)) => no_store(Json(tail).into_response()),
        Ok(Err(error)) => read_error(error),
        Err(_) => read_error(ReadError::Unavailable),
    }
}

pub async fn download(
    State(runtime): State<Arc<Runtime>>,
    query: Result<Query<DownloadQuery>, QueryRejection>,
) -> Response {
    let Ok(Query(query)) = query else {
        return read_error(ReadError::InvalidName);
    };
    let name = query.file.unwrap_or_else(|| file_name(0));
    let logs = Arc::clone(&runtime.logs);
    match tokio::task::spawn_blocking(move || logs.download(name)).await {
        Ok(Ok((name, bytes))) => {
            let mut response = bytes.into_response();
            response.headers_mut().insert(
                header::CONTENT_TYPE,
                axum::http::HeaderValue::from_static("application/x-ndjson; charset=utf-8"),
            );
            if let Ok(value) = format!("attachment; filename=\"{name}\"").parse() {
                response
                    .headers_mut()
                    .insert(header::CONTENT_DISPOSITION, value);
            }
            no_store(response)
        }
        Ok(Err(error)) => read_error(error),
        Err(_) => read_error(ReadError::Unavailable),
    }
}

fn read_error(error: ReadError) -> Response {
    let (status, code, message) = match error {
        ReadError::InvalidName => (
            StatusCode::BAD_REQUEST,
            "invalid_log_request",
            "请选择有效的日志文件和筛选条件。",
        ),
        ReadError::Missing => (
            StatusCode::NOT_FOUND,
            "log_file_not_found",
            "日志文件已轮转或不存在，请刷新文件列表。",
        ),
        ReadError::Unavailable => (
            StatusCode::SERVICE_UNAVAILABLE,
            "logs_unavailable",
            "日志文件暂不可用，请检查状态目录或容器日志。",
        ),
    };
    no_store(
        (
            status,
            Json(json!({"success":false,"error":code,"message":message})),
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
