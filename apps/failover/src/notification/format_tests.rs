use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};

use axum::{
    Json, Router,
    body::Bytes,
    extract::{OriginalUri, State},
    routing::any,
};
use serde_json::{Value, json};

use super::{Event, ProviderConfig, formatting, providers, transport::PreparedChannel};

fn sample() -> Event {
    Event {
        id: "format-check".into(),
        kind: "wan_ipv6_changed".into(),
        title: "**线路状态**".into(),
        text: "## 线路\n- **恢复**\n- [控制台](https://example.com/panel?a=1&b=2)\nIPv6：2408:abcd::1\n<状态>".into(),
        timestamp_ms: 1_799_000_000_000,
        context: super::network_context::NetworkContext {
            interface: "WAN1".into(), isp: "中国联通".into(),
            ipv4: "203.0.113.10".into(), ipv6: "2408:abcd::1".into(),
            ..Default::default()
        },
        previous_context: None,
    }
}

#[test]
fn plaintext_keeps_links_lists_and_ipv6() {
    let result = formatting::plain(&sample().text);
    assert!(result.contains("• 恢复"));
    assert!(result.contains("控制台 (https://example.com/panel?a=1&b=2)"));
    assert!(result.contains("2408:abcd::1"));
    assert!(result.contains("<状态>"));
    assert!(!result.contains("**"));
}

#[test]
fn platform_markdown_uses_supported_link_and_emphasis_syntax() {
    let slack = formatting::slack_markdown(&sample().text);
    assert!(slack.contains("*恢复*"));
    assert!(slack.contains("<https://example.com/panel?a=1&amp;b=2|控制台>"));
    let synology = formatting::synology_markdown(&sample().text);
    assert!(synology.contains("* *恢复*"));
    assert!(synology.contains("<https://example.com/panel?a=1&amp;b=2|控制台>"));
    let telegram = formatting::telegram_html(&sample().text);
    assert!(telegram.contains("<b>恢复</b>"));
    assert!(telegram.contains("href=\"https://example.com/panel?a=1&amp;b=2\""));
}

#[test]
fn linked_images_do_not_create_nested_provider_links() {
    let source = "[![封面](https://example.com/cover.jpg)](https://example.com/movie)";
    assert_eq!(
        formatting::telegram_html(source),
        "<a href=\"https://example.com/movie\">封面</a>"
    );
    for render in [formatting::slack_markdown, formatting::synology_markdown] {
        assert_eq!(render(source), "<https://example.com/movie|封面>");
    }
    assert_eq!(
        formatting::telegram_markdown_v2(source),
        "[封面](https://example.com/movie)"
    );
    let standalone = "![封面](https://example.com/cover.jpg)";
    assert_eq!(
        formatting::telegram_html(standalone),
        "<a href=\"https://example.com/cover.jpg\">封面</a>"
    );
    for render in [formatting::slack_markdown, formatting::synology_markdown] {
        assert_eq!(render(standalone), "<https://example.com/cover.jpg|封面>");
    }
}

#[test]
fn telegram_markdown_v2_escapes_text_code_and_links_in_their_own_contexts() {
    let source = "## **线路!**\n- IPv6: 2408:abcd::1!\n- IPv4: 203.0.113.10\n- 时间: 2026-11-05 19:19:47\n- [控制台](<https://example.com/a(b)>)\n\n`C:\\tmp\\a_!.log`\n\n```text\nC:\\tmp\\a_!.log `value`\n```";
    let rendered = formatting::telegram_markdown_v2(source);
    assert!(rendered.contains("*线路\\!*"));
    assert!(!rendered.contains("**"));
    assert!(rendered.contains("2408:abcd::1\\!"));
    assert!(rendered.contains("203\\.0\\.113\\.10"));
    assert!(rendered.contains("2026\\-11\\-05 19:19:47"));
    assert!(rendered.contains("[控制台](https://example.com/a(b\\))"));
    assert!(rendered.contains("`C:\\\\tmp\\\\a_!.log`"));
    assert!(rendered.contains("C:\\\\tmp\\\\a_!.log \\`value\\`"));
    assert_eq!(
        formatting::telegram_markdown_v2("**粗体 `code_!.` 结束**"),
        "*粗体 *`code_!.`* 结束*"
    );
    assert_eq!(
        formatting::telegram_markdown_v2("**粗体 `code_!.`**"),
        "*粗体 *`code_!.`"
    );
}

#[test]
fn telegram_native_html_accepts_supported_tags_and_balances_shortened_messages() {
    assert_eq!(
        formatting::telegram_native_html("<b>加粗</b> 和 **标题**"),
        "<b>加粗</b> 和 <b>标题</b>"
    );
    assert_eq!(
        formatting::telegram_native_html(
            "<a href=\"https://example.com/panel?a=1&amp;b=2\" onclick=\"run()\">控制台</a>"
        ),
        "<a href=\"https://example.com/panel?a=1&amp;b=2\">控制台</a>"
    );
    assert_eq!(
        formatting::telegram_native_html("<a href=\"javascript:run()\">点击</a>"),
        "点击"
    );
    assert_eq!(
        formatting::telegram_native_html("<b>粗体 <code>x_!.log</code> 结束</b>"),
        "<b>粗体 </b><code>x_!.log</code><b> 结束</b>"
    );
    let mut event = sample();
    event.text = format!("<b>{}", "恢复!".repeat(2000));
    let rendered =
        formatting::bounded_rendered(&event, 4000, formatting::telegram_native_html).unwrap();
    assert!(rendered.len() <= 4000);
    assert_eq!(
        rendered.matches("<b>").count(),
        rendered.matches("</b>").count()
    );
    assert!(rendered.ends_with("</b>"));
    assert!(rendered.contains("2408:abcd::1"));
}

#[test]
fn conversion_and_json_expansion_preserve_limits_and_context() {
    let mut event = sample();
    event.text = format!(
        "{}\n[控制台](https://example.com/{})",
        "<&_中文>".repeat(2000),
        "p".repeat(5000)
    );
    let plain = formatting::bounded_plain(&event, 2048).unwrap();
    assert!(plain.len() <= 2048);
    assert!(plain.contains("2408:abcd::1"));
    assert!(plain.contains("时间"));
    for render in [
        formatting::telegram_html,
        formatting::telegram_native_html,
        formatting::telegram_markdown_v2,
        formatting::slack_markdown,
        formatting::escape_markdown,
    ] {
        let result = formatting::bounded_rendered(&event, 4000, render).unwrap();
        assert!(result.len() <= 4000);
        assert!(result.contains("2408:abcd::1"));
    }
    event.text = "\"\\\n".repeat(3000);
    let push = providers::webpush_content(&event, true).unwrap();
    assert!(push.len() <= 3500);
    let payload: Value = serde_json::from_str(&push).unwrap();
    assert_eq!(payload["message_format"], "text");
    assert!(payload["body"].as_str().unwrap().contains("2408:abcd::1"));
}

async fn receive(
    State(records): State<Arc<Mutex<Vec<Value>>>>,
    OriginalUri(uri): OriginalUri,
    body: Bytes,
) -> Json<Value> {
    if !body.is_empty() {
        let value: Value = if body.starts_with(b"payload=") {
            reqwest::Url::parse(&format!(
                "http://localhost/?{}",
                String::from_utf8_lossy(&body)
            ))
            .unwrap()
            .query_pairs()
            .find(|(key, _)| key == "payload")
            .map(|(_, payload)| serde_json::from_str(&payload).unwrap())
            .unwrap()
        } else if let Ok(value) = serde_json::from_slice(&body) {
            value
        } else {
            json!({"content": String::from_utf8_lossy(&body)})
        };
        records.lock().unwrap().push(value);
    }
    if uri.path().contains("/channels/") || uri.path().starts_with("/v2/") {
        Json(json!({"id":"mock-id"}))
    } else {
        Json(
            json!({"errcode":0,"access_token":"mock","tenant_access_token":"mock","code":0,"ok":true,"id":"mock-id","success":true,"accepted":1,"failed":[]}),
        )
    }
}

#[tokio::test]
async fn providers_use_client_compatible_payloads() {
    let records = Arc::new(Mutex::new(Vec::new()));
    let app = Router::new()
        .fallback(any(receive))
        .with_state(records.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let client = super::transport::client(5000).unwrap();
    let mut channel = PreparedChannel {
        provider: ProviderConfig::Wechat {
            api_url: base.clone(),
            corp_id: "corp".into(),
            corp_secret_env: "SECRET".into(),
            agent_id: 1,
            touser: "user".into(),
        },
        secrets: HashMap::from([
            ("SECRET".into(), "mock".into()),
            ("HOOK".into(), base.clone()),
        ]),
        mp_config: BTreeMap::new(),
        message_format: "markdown".into(),
    };
    providers::send(&client, &channel, &sample()).await.unwrap();
    let payload = records.lock().unwrap().pop().unwrap();
    assert_eq!(payload["msgtype"], "text");
    assert!(
        payload["text"]["content"]
            .as_str()
            .unwrap()
            .contains("https://example.com/panel")
    );
    channel
        .mp_config
        .insert("WECHAT_APP_MARKDOWN".into(), json!(true));
    providers::send(&client, &channel, &sample()).await.unwrap();
    assert_eq!(
        records.lock().unwrap().pop().unwrap()["msgtype"],
        "markdown"
    );

    channel.mp_config.insert("QQ_MARKDOWN".into(), json!(false));
    let cases = [
        (
            ProviderConfig::QyWechatBot {
                webhook_url_env: "HOOK".into(),
                message_type: "markdown".into(),
            },
            "msgtype",
            "markdown",
        ),
        (
            ProviderConfig::Telegram {
                api_url: base.clone(),
                bot_token_env: "SECRET".into(),
                chat_id: "user".into(),
                message_thread_id: None,
            },
            "parse_mode",
            "MarkdownV2",
        ),
        (
            ProviderConfig::Slack {
                api_url: base.clone(),
                bot_token_env: "SECRET".into(),
                channel: "channel".into(),
            },
            "text",
            "",
        ),
        (
            ProviderConfig::Discord {
                api_url: base.clone(),
                bot_token_env: "SECRET".into(),
                channel_id: "channel".into(),
            },
            "content",
            "",
        ),
        (
            ProviderConfig::DingTalk {
                webhook_url_env: "HOOK".into(),
                secret_env: None,
            },
            "msgtype",
            "markdown",
        ),
        (
            ProviderConfig::SynologyChat {
                webhook_url_env: "HOOK".into(),
                user_ids: vec![],
            },
            "text",
            "",
        ),
        (
            ProviderConfig::VoceChat {
                api_url: base.clone(),
                api_key_env: "SECRET".into(),
                target: "GID#1".into(),
            },
            "content",
            "",
        ),
        (
            ProviderConfig::Feishu {
                api_url: base.clone(),
                app_id: "app".into(),
                app_secret_env: "SECRET".into(),
                receive_id: "chat".into(),
                receive_id_type: "chat_id".into(),
            },
            "msg_type",
            "interactive",
        ),
        (
            ProviderConfig::Qq {
                api_url: base.clone(),
                token_url: base.clone(),
                app_id: "app".into(),
                app_secret_env: "SECRET".into(),
                recipient: "user".into(),
                recipient_type: "user".into(),
            },
            "msg_type",
            "",
        ),
    ];
    for (provider, key, expected) in cases {
        let provider_name = provider.name();
        channel.provider = provider;
        providers::send(&client, &channel, &sample()).await.unwrap();
        let payload = records.lock().unwrap().pop().unwrap();
        assert!(payload.get(key).is_some(), "{provider_name}: {payload}");
        if !expected.is_empty() {
            assert_eq!(payload[key], expected, "{provider_name}");
        }
        if provider_name == "slack" || provider_name == "synology_chat" {
            assert!(payload["text"].as_str().unwrap().contains("|控制台>"));
        }
        if provider_name == "qq" {
            assert_eq!(payload["msg_type"], 0);
            assert!(!payload["content"].as_str().unwrap().contains("**"));
        }
        records.lock().unwrap().clear();
    }
    channel.mp_config.insert("QQ_MARKDOWN".into(), json!(true));
    providers::send(&client, &channel, &sample()).await.unwrap();
    assert_eq!(records.lock().unwrap().pop().unwrap()["msg_type"], 2);
    server.abort();
}

#[tokio::test]
async fn app_push_custom_templates_keep_network_context_when_shortened() {
    let records = Arc::new(Mutex::new(Vec::new()));
    let app = Router::new()
        .fallback(any(receive))
        .with_state(records.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let client = super::transport::client(5000).unwrap();
    let mut channel = PreparedChannel {
        provider: ProviderConfig::AppPush {
            api_url: base,
            relay_key_env: "RELAY".into(),
            owner: "owner".into(),
            route: "/logs/notifications".into(),
        },
        secrets: HashMap::from([("RELAY".into(), "r".repeat(32))]),
        mp_config: BTreeMap::new(),
        message_format: "markdown".into(),
    };
    let mut event = sample();
    event.previous_context = Some(super::network_context::NetworkContext {
        interface: "WAN3".into(),
        isp: "中国移动".into(),
        ipv4: "198.51.100.20".into(),
        ipv6: "2001:db8::20".into(),
        ..Default::default()
    });
    // A custom template deliberately omits context and exceeds the platform limit.
    event.text = format!(
        "[控制台](https://example.com/panel)\n{}",
        "**恢复**\n".repeat(10_000)
    );
    for format in ["markdown", "text"] {
        channel.message_format = format.into();
        providers::send(&client, &channel, &event).await.unwrap();
        let payload = records.lock().unwrap().pop().unwrap();
        let body = payload["body"].as_str().unwrap();
        assert!(body.len() <= 30_000);
        assert!(body.contains("正文已缩短"));
        for value in [
            "WAN1",
            "WAN3",
            "中国联通",
            "中国移动",
            "203.0.113.10",
            "198.51.100.20",
            "2408:abcd::1",
            "2001:db8::20",
            "时间",
            "https://example.com/panel",
        ] {
            assert!(body.contains(value), "{format}: missing {value}");
        }
        assert_eq!(payload["title"], "线路状态");
        assert!(!body.contains("**"));
        assert!(!body.starts_with(payload["title"].as_str().unwrap()));
    }
    server.abort();
}
