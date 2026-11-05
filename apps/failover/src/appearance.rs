use axum::{
    Json,
    http::{StatusCode, header},
    response::{IntoResponse, Response},
};
use serde_json::{Value, json};
use std::{
    sync::OnceLock,
    time::{Duration, Instant},
};
use tokio::sync::Mutex;

struct Wallpaper {
    loaded: Instant,
    image: Vec<u8>,
    mime: String,
    copyright: String,
}
static WALLPAPER: OnceLock<Mutex<Option<Wallpaper>>> = OnceLock::new();

async fn refresh() -> Result<Wallpaper, ()> {
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(15))
        .build()
        .map_err(|_| ())?;
    let response = client
        .get("https://www.bing.com/HPImageArchive.aspx")
        .query(&[("format", "js"), ("idx", "0"), ("n", "1"), ("mkt", "zh-CN")])
        .send()
        .await
        .map_err(|_| ())?;
    if !response.status().is_success() {
        return Err(());
    }
    let value: Value =
        serde_json::from_slice(&limited(response, 256 * 1024).await?).map_err(|_| ())?;
    let image = value["images"]
        .as_array()
        .and_then(|images| images.first())
        .ok_or(())?;
    let path = image["url"].as_str().ok_or(())?;
    let url = reqwest::Url::parse(&format!("https://www.bing.com{path}")).map_err(|_| ())?;
    if url.host_str() != Some("www.bing.com") || url.path() != "/th" {
        return Err(());
    }
    let response = client.get(url).send().await.map_err(|_| ())?;
    if !response.status().is_success() {
        return Err(());
    }
    let mime = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("image/jpeg")
        .to_string();
    if !["image/jpeg", "image/png", "image/webp"]
        .iter()
        .any(|kind| mime.starts_with(kind))
    {
        return Err(());
    }
    Ok(Wallpaper {
        loaded: Instant::now(),
        image: limited(response, 16 * 1024 * 1024).await?,
        mime,
        copyright: image["copyright"]
            .as_str()
            .unwrap_or("Bing 每日壁纸")
            .into(),
    })
}

async fn limited(mut response: reqwest::Response, limit: usize) -> Result<Vec<u8>, ()> {
    if response
        .content_length()
        .is_some_and(|length| length > limit as u64)
    {
        return Err(());
    }
    let mut data = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| ())? {
        if data.len().saturating_add(chunk.len()) > limit {
            return Err(());
        }
        data.extend_from_slice(&chunk);
    }
    Ok(data)
}

pub async fn wallpaper() -> Response {
    serve(false).await
}
pub async fn wallpaper_image() -> Response {
    serve(true).await
}

async fn serve(image: bool) -> Response {
    let mut stored = WALLPAPER.get_or_init(|| Mutex::new(None)).lock().await;
    if stored
        .as_ref()
        .is_none_or(|wallpaper| wallpaper.loaded.elapsed() >= Duration::from_secs(6 * 60 * 60))
    {
        if let Ok(wallpaper) = refresh().await {
            *stored = Some(wallpaper);
        }
    }
    let Some(wallpaper) = stored.as_ref() else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({"message":"暂时无法读取 Bing 每日壁纸。"})),
        )
            .into_response();
    };
    if !image {
        return Json(
            json!({"image_url":"/appearance/wallpaper/image","copyright":wallpaper.copyright}),
        )
        .into_response();
    }
    let mut response = wallpaper.image.clone().into_response();
    if let Ok(mime) = wallpaper.mime.parse() {
        response.headers_mut().insert(header::CONTENT_TYPE, mime);
    }
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        axum::http::HeaderValue::from_static("public, max-age=3600"),
    );
    response
}
