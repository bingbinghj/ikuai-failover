use serde_json::{Value, json};

fn text(value: &Value) -> Option<String> {
    value
        .as_str()
        .map(str::trim)
        .filter(|text| !text.is_empty() && text.len() <= 256 && !text.chars().any(char::is_control))
        .map(str::to_owned)
}

fn number(value: &Value) -> Option<f64> {
    value
        .as_f64()
        .or_else(|| value.as_str()?.trim().trim_end_matches('%').parse().ok())
        .filter(|value| value.is_finite() && *value >= 0.0)
}

fn percentage(value: &Value) -> Option<f64> {
    number(value).filter(|value| *value <= 100.0)
}

/// Select only dashboard fields. Router identifiers, serial numbers and addresses are not exposed.
pub(crate) fn normalize(value: &Value) -> Option<Value> {
    let root = value
        .get("sysstat")
        .or_else(|| value.get("sysinfo"))
        .unwrap_or(value);
    let mut info = serde_json::Map::new();
    for (field, input) in [
        ("hostname", &root["hostname"]),
        ("version", &root["verinfo"]["verstring"]),
        ("model", &root["verinfo"]["modelname"]),
    ] {
        if let Some(value) = text(input) {
            info.insert(field.into(), json!(value));
        }
    }
    if !info.contains_key("version") {
        if let Some(value) = text(&root["verinfo"]["version"]) {
            info.insert("version".into(), json!(value));
        }
    }
    if let Some(seconds) = number(&root["uptime"]).filter(|value| *value <= u32::MAX as f64) {
        info.insert("uptime_seconds".into(), json!(seconds as u64));
    }
    if let Some(value) = percentage(&root["cpu"][0]) {
        info.insert("cpu_percent".into(), json!(value));
    }
    if let Some(value) = percentage(&root["memory"]["used"]) {
        info.insert("memory_percent".into(), json!(value));
    }
    if let Some(value) = number(&root["memory"]["total"]) {
        info.insert("memory_total_kib".into(), json!(value));
    }
    (!info.is_empty()).then_some(Value::Object(info))
}

pub(crate) fn add_monitor_sample(info: &mut Value, monitor: &Value) {
    let Some(sample) = monitor
        .get("data")
        .and_then(Value::as_array)
        .and_then(|rows| {
            rows.iter().max_by(|a, b| {
                number(&a["timestamp"])
                    .unwrap_or_default()
                    .total_cmp(&number(&b["timestamp"]).unwrap_or_default())
            })
        })
    else {
        return;
    };
    if info.get("cpu_percent").is_none() {
        if let Some(value) = percentage(&sample["cpu"]) {
            info["cpu_percent"] = json!(value);
        }
    }
    if info.get("memory_percent").is_none() {
        if let Some(value) = percentage(&sample["memory_use"]) {
            info["memory_percent"] = json!(value);
        }
    }
    if let Some(value) = number(&sample["timestamp"]) {
        info["usage_sampled_at"] = json!(value);
    }
}
