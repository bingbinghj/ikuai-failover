use reqwest::Client;
use serde_json::{Value, json};

use super::network_context::{LineContext, available};
use super::transport::{PreparedChannel, accept, endpoint, json_response};
use super::{Event, NotificationError, ProviderConfig, formatting};

pub(crate) async fn send(
    client: &Client,
    channel: &PreparedChannel,
    event: &Event,
) -> Result<(), NotificationError> {
    let ProviderConfig::Wechat {
        api_url,
        corp_id,
        corp_secret_env,
        agent_id,
        touser,
    } = &channel.provider
    else {
        return Err(NotificationError::InvalidConfiguration);
    };
    let result = json_response(client.get(endpoint(api_url, "cgi-bin/gettoken")).query(&[
        ("corpid", corp_id.as_str()),
        ("corpsecret", channel.secret(corp_secret_env)?),
    ]))
    .await?;
    accept(result["errcode"] == 0)?;
    let token = result["access_token"]
        .as_str()
        .ok_or(NotificationError::InvalidResponse)?;
    let markdown = channel.message_format == "markdown";
    let news = channel.message_format == "news";
    let native = markdown
        && channel
            .mp_config
            .get("WECHAT_APP_MARKDOWN")
            .and_then(Value::as_bool)
            == Some(true);
    let text_body = || -> Result<Value, NotificationError> {
        let content = if markdown || news {
            formatting::bounded_plain(event, 2048)?
        } else {
            formatting::bounded(event, 2048)?
        };
        Ok(
            json!({"touser":touser,"agentid":agent_id,"msgtype":"text","text":{"content":content},"safe":0}),
        )
    };
    let body = if native {
        // Application Markdown is limited to 2048 bytes; only group webhooks allow 4096.
        json!({"touser":touser,"agentid":agent_id,"msgtype":"markdown","markdown":{"content":formatting::bounded(event,2048)?}})
    } else if news {
        match article(event) {
            Ok(article) => {
                json!({"touser":touser,"agentid":agent_id,"msgtype":"news","news":{"articles":[article]}})
            }
            Err(NotificationError::InvalidConfiguration) => {
                crate::logging::warn("[通知] 企业微信图文摘要无法完整容纳线路信息，改为兼容文本。");
                text_body()?
            }
            Err(error) => return Err(error),
        }
    } else {
        // WeChat's micro-workplace cannot display native application Markdown.
        text_body()?
    };
    let url = endpoint(api_url, "cgi-bin/message/send");
    let result = json_response(
        client
            .post(&url)
            .query(&[("access_token", token)])
            .json(&body),
    )
    .await?;
    // MP permits a null link, although the documented news contract requires a URL or mini-program.
    // Retry only explicit format/content rejections, never ambiguous network or recipient failures.
    if body["msgtype"] == "news"
        && result["errcode"].as_i64().is_some_and(|code| {
            matches!(code, 40008 | 40033 | 40058 | 44003 | 45002 | 45003 | 45004)
        })
    {
        crate::logging::warn("[通知] 企业微信接口拒绝无链接图文消息，改为兼容文本。");
        let result = json_response(
            client
                .post(&url)
                .query(&[("access_token", token)])
                .json(&text_body()?),
        )
        .await?;
        return accepted(&result);
    }
    accepted(&result)
}

fn accepted(result: &Value) -> Result<(), NotificationError> {
    accept(
        result["errcode"] == 0
            && ["invaliduser", "invalidparty", "invalidtag"]
                .iter()
                .all(|key| result[*key].as_str().is_none_or(str::is_empty)),
    )
}

fn prefix(value: &str, limit: usize) -> &str {
    let mut end = value.len().min(limit);
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    &value[..end]
}

fn line_summary(label: &str, line: &LineContext) -> String {
    let identity = [&line.interface, &line.isp]
        .into_iter()
        .filter(|value| available(value))
        .map(String::as_str)
        .collect::<Vec<_>>()
        .join(" · ");
    let mut rows = Vec::new();
    if !identity.is_empty() {
        rows.push(format!("{label}：{identity}"));
    }
    for (key, value) in [
        ("IPv4", &line.ipv4),
        ("IPv6", &line.ipv6),
        ("PD", &line.prefix),
    ] {
        if available(value) {
            rows.push(format!("{key}：{value}"));
        }
    }
    rows.join("\n")
}

pub(crate) fn article(event: &Event) -> Result<Value, NotificationError> {
    let title = formatting::plain(&event.title);
    let title = prefix(title.trim(), 128);
    if title.is_empty() {
        return Err(NotificationError::InvalidConfiguration);
    }
    let mut context = event.context.clone();
    if let Some(previous) = event.previous_context.as_ref() {
        context = context.with_transition(previous, &event.context.affected_interface);
    }
    // Avoid duplicating the active line after a transition, keeping both actual snapshots intact.
    let mut rows = if context.previous.is_some() {
        let (before, after) = context.transition_lines();
        let mut rows = vec![line_summary("之前", &before), line_summary("之后", &after)];
        let active = context.active_line();
        if !active.interface.eq_ignore_ascii_case(&after.interface) {
            rows.push(line_summary("当前", &active));
        }
        for line in &context.lines {
            if !line.interface.eq_ignore_ascii_case(&after.interface)
                && !line.interface.eq_ignore_ascii_case(&active.interface)
            {
                rows.push(line_summary("线路", line));
            }
        }
        rows
    } else {
        let mut rows = vec![line_summary("当前", &context.active_line())];
        for line in &context.lines {
            if !line.interface.eq_ignore_ascii_case(&context.interface) {
                rows.push(line_summary("线路", line));
            }
        }
        rows
    };
    rows.retain(|row| !row.is_empty());
    if context.stale {
        rows.push("地址为上次记录".into());
    }
    let time = super::templates::notification_timestamp(event.timestamp_ms);
    rows.push(format!("时间：{time}"));
    let footer = rows.join("\n");
    if footer.len() > 512 {
        return Err(NotificationError::InvalidConfiguration);
    }
    let body = event
        .text
        .replace(&super::network_context::summary(&context), "")
        .replace(&super::network_context::transition_summary(&context), "")
        .replace(&super::network_context::current_summary(&context), "")
        .replace(&format!("🕒 时间：{time}"), "");
    let body = formatting::plain(body.trim());
    let maximum = 512usize.saturating_sub(footer.len() + 2);
    let marker = "\n…（正文已缩短）";
    let body = if body.len() <= maximum {
        body
    } else if maximum >= marker.len() {
        format!("{}{marker}", prefix(&body, maximum - marker.len()))
    } else {
        String::new()
    };
    let description = if body.is_empty() {
        footer
    } else {
        format!("{body}\n\n{footer}")
    };
    // Match MP's absent-link payload; do not invent a URL or require a cover image.
    Ok(json!({"title":title,"description":description,"url":null}))
}
