use super::{Event, NotificationError};
use pulldown_cmark::{Event as MarkdownEvent, Options, Parser, Tag, TagEnd};

/// Reserve required metadata before shortening a message for a provider's byte limit.
pub fn bounded(event: &Event, limit: usize) -> Result<String, NotificationError> {
    bounded_with(event, limit, str::to_owned)
}

/// Convert before truncation so Markdown links cannot enlarge an already bounded payload.
pub fn bounded_plain(event: &Event, limit: usize) -> Result<String, NotificationError> {
    bounded_with(event, limit, plain)
}

/// Bound the rendered provider payload too; entity escaping can multiply its byte size.
pub fn bounded_rendered(
    event: &Event,
    limit: usize,
    render: impl Fn(&str) -> String,
) -> Result<String, NotificationError> {
    let mut source_limit = limit;
    for _ in 0..16 {
        let content = render(&bounded(event, source_limit)?);
        if content.len() <= limit {
            return Ok(content);
        }
        source_limit = source_limit.saturating_mul(limit) / content.len();
        source_limit = source_limit.saturating_sub(32);
    }
    Err(NotificationError::InvalidConfiguration)
}

fn bounded_with(
    event: &Event,
    limit: usize,
    render: impl Fn(&str) -> String,
) -> Result<String, NotificationError> {
    let context = if let Some(previous) = event.previous_context.as_ref() {
        event
            .context
            .clone()
            .with_transition(previous, &event.context.affected_interface)
    } else {
        event.context.clone()
    };
    let summary = super::network_context::summary(&context);
    let transition = super::network_context::transition_summary(&context);
    let current = super::network_context::current_summary(&context);
    let time = super::templates::notification_timestamp(event.timestamp_ms);
    let footer = render(&format!("{summary}\n🕒 时间：{time}"));
    if footer.len() >= limit {
        return Err(NotificationError::InvalidConfiguration);
    }
    let body = event
        .text
        .replace(&summary, "")
        .replace(&transition, "")
        .replace(&current, "")
        .replace(&format!("🕒 时间：{time}"), "");
    let text = format!("{}\n{}", render(&event.title), render(body.trim()));
    let separator = "\n\n";
    let maximum = limit.saturating_sub(footer.len() + separator.len());
    let text = truncate(&text, maximum);
    Ok(format!("{text}{separator}{footer}"))
}

fn truncate(value: &str, limit: usize) -> String {
    if value.len() <= limit {
        return value.into();
    }
    let marker = "\n…（正文已缩短）";
    let mut end = limit.saturating_sub(marker.len());
    while end > 0 && !value.is_char_boundary(end) {
        end -= 1;
    }
    if marker.len() > limit {
        return value[..end].into();
    }
    format!("{}{marker}", &value[..end])
}

pub fn plain(source: &str) -> String {
    let mut text = String::new();
    let mut links = Vec::new();
    for event in Parser::new_ext(source, Options::ENABLE_STRIKETHROUGH) {
        match event {
            MarkdownEvent::Text(value) | MarkdownEvent::Code(value) => text.push_str(&value),
            MarkdownEvent::Html(value) | MarkdownEvent::InlineHtml(value) => text.push_str(&value),
            MarkdownEvent::SoftBreak | MarkdownEvent::HardBreak => text.push('\n'),
            MarkdownEvent::Start(Tag::Item) => text.push_str("• "),
            MarkdownEvent::Start(Tag::Link { dest_url, .. } | Tag::Image { dest_url, .. }) => {
                links.push((text.len(), dest_url.into_string()));
            }
            MarkdownEvent::End(TagEnd::Link | TagEnd::Image) => {
                if let Some((start, url)) = links.pop() {
                    if !url.is_empty() && text[start..].trim() != url {
                        if text.len() > start {
                            text.push_str(" (");
                            text.push_str(&url);
                            text.push(')');
                        } else {
                            text.push_str(&url);
                        }
                    }
                }
            }
            MarkdownEvent::End(
                TagEnd::Paragraph | TagEnd::Heading(_) | TagEnd::Item | TagEnd::CodeBlock,
            ) => text.push('\n'),
            MarkdownEvent::Rule => text.push_str("\n────\n"),
            _ => {}
        }
    }
    text.trim().to_string()
}

fn escape_html(source: &str) -> String {
    source
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn escape_telegram_text(source: &str) -> String {
    let mut result = String::new();
    for character in source.chars() {
        if "\\_*[]()~`>#+-=|{}.!".contains(character) {
            result.push('\\');
        }
        result.push(character);
    }
    result
}

fn escape_telegram_code(source: &str) -> String {
    source.replace('\\', "\\\\").replace('`', "\\`")
}

fn resume_telegram_styles(result: &mut String, styles: &mut [(char, bool, bool)]) {
    for (marker, owns, open) in styles {
        if *owns && !*open {
            result.push(*marker);
            *open = true;
        }
    }
}

fn suspend_telegram_styles(result: &mut String, styles: &mut [(char, bool, bool)]) {
    for (marker, _, open) in styles.iter_mut().rev() {
        if *open {
            result.push(*marker);
            *open = false;
        }
    }
}

/// Render CommonMark using Telegram's MarkdownV2 entities and context-specific escapes.
pub fn telegram_markdown_v2(source: &str) -> String {
    let mut result = String::new();
    let mut links: Vec<(bool, String)> = Vec::new();
    let mut styles: Vec<(char, bool, bool)> = Vec::new();
    let mut code_block = false;
    for event in Parser::new_ext(source, Options::ENABLE_STRIKETHROUGH) {
        match event {
            MarkdownEvent::Text(value) => {
                if code_block {
                    result.push_str(&escape_telegram_code(&value));
                } else {
                    resume_telegram_styles(&mut result, &mut styles);
                    result.push_str(&escape_telegram_text(&value));
                }
            }
            MarkdownEvent::Html(value) | MarkdownEvent::InlineHtml(value) => {
                resume_telegram_styles(&mut result, &mut styles);
                result.push_str(&escape_telegram_text(&value));
            }
            MarkdownEvent::Code(value) => {
                if links.iter().any(|(active, _)| *active) {
                    resume_telegram_styles(&mut result, &mut styles);
                    result.push_str(&escape_telegram_text(&value));
                } else {
                    suspend_telegram_styles(&mut result, &mut styles);
                    result.push('`');
                    result.push_str(&escape_telegram_code(&value));
                    result.push('`');
                }
            }
            MarkdownEvent::SoftBreak | MarkdownEvent::HardBreak => {
                if !code_block {
                    resume_telegram_styles(&mut result, &mut styles);
                }
                result.push('\n');
            }
            MarkdownEvent::Start(tag) => match tag {
                Tag::Strong | Tag::Heading { .. } | Tag::Emphasis | Tag::Strikethrough => {
                    let marker = match tag {
                        Tag::Emphasis => '_',
                        Tag::Strikethrough => '~',
                        _ => '*',
                    };
                    resume_telegram_styles(&mut result, &mut styles);
                    let active = !styles
                        .iter()
                        .any(|(existing, owns, _)| *existing == marker && *owns);
                    styles.push((marker, active, active));
                    if active {
                        result.push(marker);
                    }
                }
                Tag::CodeBlock(_) => {
                    suspend_telegram_styles(&mut result, &mut styles);
                    result.push_str("```\n");
                    code_block = true;
                }
                Tag::Item => result.push_str("• "),
                Tag::Link { dest_url, .. } | Tag::Image { dest_url, .. } => {
                    resume_telegram_styles(&mut result, &mut styles);
                    let active = !links.iter().any(|(active, _)| *active)
                        && reqwest::Url::parse(&dest_url)
                            .is_ok_and(|url| ["http", "https"].contains(&url.scheme()));
                    links.push((active, dest_url.into_string()));
                    if active {
                        result.push('[');
                    }
                }
                _ => {}
            },
            MarkdownEvent::End(tag) => match tag {
                TagEnd::Strong | TagEnd::Heading(_) | TagEnd::Emphasis | TagEnd::Strikethrough => {
                    if let Some((marker, _, true)) = styles.pop() {
                        result.push(marker);
                    }
                    if matches!(tag, TagEnd::Heading(_)) {
                        result.push('\n');
                    }
                }
                TagEnd::CodeBlock => {
                    result.push_str("\n```\n");
                    code_block = false;
                }
                TagEnd::Link | TagEnd::Image => {
                    if let Some((true, url)) = links.pop() {
                        result.push_str("](");
                        result.push_str(&url.replace('\\', "\\\\").replace(')', "\\)"));
                        result.push(')');
                    }
                }
                TagEnd::Paragraph | TagEnd::Item => result.push('\n'),
                _ => {}
            },
            MarkdownEvent::Rule => result.push_str("\n────\n"),
            _ => {}
        }
    }
    result.trim().to_owned()
}

/// Telegram supports a small HTML subset; parse Markdown into that supported subset.
#[cfg(test)]
pub fn telegram_html(source: &str) -> String {
    normalize_telegram_html(&telegram_render(source, false))
}

/// HTML mode also accepts native Telegram HTML, alongside Markdown templates.
pub fn telegram_native_html(source: &str) -> String {
    normalize_telegram_html(&telegram_render(source, true))
}

fn telegram_render(source: &str, allow_html: bool) -> String {
    let mut result = String::new();
    let mut link_stack = Vec::new();
    for event in Parser::new_ext(source, Options::ENABLE_STRIKETHROUGH) {
        match event {
            MarkdownEvent::Text(value) => result.push_str(&escape_html(&value)),
            MarkdownEvent::Html(value) | MarkdownEvent::InlineHtml(value) => {
                if allow_html {
                    result.push_str(&value);
                } else {
                    result.push_str(&escape_html(&value));
                }
            }
            MarkdownEvent::Code(value) => {
                result.push_str(&format!("<code>{}</code>", escape_html(&value)))
            }
            MarkdownEvent::SoftBreak | MarkdownEvent::HardBreak => result.push('\n'),
            MarkdownEvent::Start(tag) => match tag {
                Tag::Strong | Tag::Heading { .. } => result.push_str("<b>"),
                Tag::Emphasis => result.push_str("<i>"),
                Tag::Strikethrough => result.push_str("<s>"),
                Tag::CodeBlock(_) => result.push_str("<pre>"),
                Tag::Item => result.push_str("• "),
                Tag::Link { dest_url, .. } | Tag::Image { dest_url, .. } => {
                    // Images inside a link contribute their alt text, not a nested anchor.
                    let valid = !link_stack.iter().any(|active| *active)
                        && reqwest::Url::parse(&dest_url)
                            .is_ok_and(|url| ["http", "https"].contains(&url.scheme()));
                    link_stack.push(valid);
                    if valid {
                        result.push_str(&format!("<a href=\"{}\">", escape_html(&dest_url)));
                    }
                }
                _ => {}
            },
            MarkdownEvent::End(tag) => match tag {
                TagEnd::Strong => result.push_str("</b>"),
                TagEnd::Heading(_) => result.push_str("</b>\n"),
                TagEnd::Emphasis => result.push_str("</i>"),
                TagEnd::Strikethrough => result.push_str("</s>"),
                TagEnd::CodeBlock => result.push_str("</pre>\n"),
                TagEnd::Link | TagEnd::Image => {
                    if link_stack.pop() == Some(true) {
                        result.push_str("</a>");
                    }
                }
                TagEnd::Paragraph | TagEnd::Item => result.push('\n'),
                _ => {}
            },
            MarkdownEvent::Rule => result.push_str("\n────\n"),
            _ => {}
        }
    }
    result.trim().to_string()
}

fn decode_html_entities(source: &str) -> String {
    let mut result = String::new();
    let mut position = 0;
    while position < source.len() {
        let remaining = &source[position..];
        if remaining.starts_with('&') {
            if let Some(end) = remaining.find(';').filter(|end| *end <= 16) {
                let name = &remaining[1..end];
                let value = match name {
                    "amp" => Some('&'),
                    "lt" => Some('<'),
                    "gt" => Some('>'),
                    "quot" => Some('"'),
                    "apos" => Some('\''),
                    _ => name
                        .strip_prefix("#x")
                        .or_else(|| name.strip_prefix("#X"))
                        .and_then(|number| u32::from_str_radix(number, 16).ok())
                        .or_else(|| {
                            name.strip_prefix('#')
                                .and_then(|number| number.parse().ok())
                        })
                        .and_then(char::from_u32),
                };
                if let Some(value) = value {
                    result.push(value);
                    position += end + 1;
                    continue;
                }
            }
        }
        let character = remaining.chars().next().unwrap();
        result.push(character);
        position += character.len_utf8();
    }
    result
}

fn html_href(attributes: &str) -> Option<String> {
    let mut remaining = attributes.trim();
    while !remaining.is_empty() {
        let end = remaining
            .find(|character: char| character.is_whitespace() || character == '=')
            .unwrap_or(remaining.len());
        let key = &remaining[..end];
        remaining = remaining[end..].trim_start();
        if !remaining.starts_with('=') {
            break;
        }
        remaining = remaining[1..].trim_start();
        let (value, after) = if remaining.starts_with('\'') || remaining.starts_with('"') {
            let quote = remaining.chars().next().unwrap();
            let end = remaining[1..].find(quote)? + 1;
            (&remaining[1..end], &remaining[end + 1..])
        } else {
            let end = remaining
                .find(char::is_whitespace)
                .unwrap_or(remaining.len());
            (&remaining[..end], &remaining[end..])
        };
        if key.eq_ignore_ascii_case("href") {
            return Some(decode_html_entities(value));
        }
        remaining = after.trim_start();
    }
    None
}

struct HtmlFrame {
    name: String,
    opening: String,
    emitted: bool,
    restore: Vec<usize>,
}

fn close_html_frame(result: &mut String, frames: &mut Vec<HtmlFrame>, restore: bool) {
    let frame = frames.pop().unwrap();
    if frame.emitted {
        result.push_str(&format!("</{}>", frame.name));
    }
    if restore {
        for index in frame.restore {
            if let Some(parent) = frames.get_mut(index) {
                result.push_str(&parent.opening);
                parent.emitted = true;
            }
        }
    }
}

/// Whitelist tags, avoid nested links/code entities, and close tags cut by the byte limit.
fn normalize_telegram_html(source: &str) -> String {
    let mut result = String::new();
    let mut frames: Vec<HtmlFrame> = Vec::new();
    let mut position = 0;
    while position < source.len() {
        let remaining = &source[position..];
        let Some(start) = remaining.find('<') else {
            result.push_str(&escape_html(&decode_html_entities(remaining)));
            break;
        };
        result.push_str(&escape_html(&decode_html_entities(&remaining[..start])));
        position += start;
        let mut quote = None;
        let end = source[position + 1..]
            .char_indices()
            .find_map(|(index, character)| {
                if let Some(active) = quote {
                    if character == active {
                        quote = None;
                    }
                } else if ['\'', '"'].contains(&character) {
                    quote = Some(character);
                } else if character == '>' {
                    return Some(position + index + 2);
                }
                None
            });
        let Some(end) = end else {
            result.push_str(&escape_html(&decode_html_entities(&source[position..])));
            break;
        };
        let raw = &source[position..end];
        let token = source[position + 1..end - 1].trim();
        position = end;
        let closing = token.starts_with('/');
        let token = token.trim_start_matches('/');
        let name_end = token
            .find(|character: char| !character.is_ascii_alphabetic() && character != '-')
            .unwrap_or(token.len());
        let name = match token[..name_end].to_ascii_lowercase().as_str() {
            "b" | "strong" => "b",
            "i" | "em" => "i",
            "u" | "ins" => "u",
            "s" | "strike" | "del" => "s",
            "code" => "code",
            "pre" => "pre",
            "tg-spoiler" => "tg-spoiler",
            "a" => "a",
            "br" => {
                result.push('\n');
                continue;
            }
            _ => {
                result.push_str(&escape_html(&decode_html_entities(raw)));
                continue;
            }
        };
        let code_parent = frames
            .iter()
            .rfind(|frame| frame.emitted && ["pre", "code"].contains(&frame.name.as_str()));
        if let Some(parent) = code_parent {
            if !(closing && ["pre", "code"].contains(&name))
                && !(parent.name == "pre" && name == "code" && !closing)
            {
                result.push_str(&escape_html(&decode_html_entities(raw)));
                continue;
            }
        }
        if closing {
            if let Some(index) = frames.iter().rposition(|frame| frame.name == name) {
                while frames.len() > index {
                    close_html_frame(&mut result, &mut frames, true);
                }
            }
            continue;
        }
        let href = if name == "a" {
            html_href(&token[name_end..])
        } else {
            None
        };
        let opening = if name == "a" {
            href.filter(|url| {
                reqwest::Url::parse(url).is_ok_and(|url| ["http", "https"].contains(&url.scheme()))
            })
            .map(|url| format!("<a href=\"{}\">", escape_html(&url)))
            .unwrap_or_default()
        } else {
            format!("<{name}>")
        };
        let mut restore = Vec::new();
        if ["pre", "code"].contains(&name) && code_parent.is_none() {
            for (index, parent) in frames.iter_mut().enumerate().rev() {
                if parent.emitted {
                    result.push_str(&format!("</{}>", parent.name));
                    parent.emitted = false;
                    restore.push(index);
                }
            }
            restore.reverse();
        }
        let emitted = !opening.is_empty()
            && !frames
                .iter()
                .any(|frame| frame.name == name && frame.emitted);
        if emitted {
            result.push_str(&opening);
        }
        frames.push(HtmlFrame {
            name: name.into(),
            opening,
            emitted,
            restore,
        });
        if token.ends_with('/') {
            close_html_frame(&mut result, &mut frames, true);
        }
    }
    while !frames.is_empty() {
        close_html_frame(&mut result, &mut frames, false);
    }
    result.trim().to_owned()
}

pub fn escape_markdown(source: &str) -> String {
    let mut result = String::new();
    for character in source.chars() {
        if "\\*_~`[]()#>!|".contains(character) {
            result.push('\\');
        }
        result.push(character);
    }
    result
}

pub fn slack_markdown(source: &str) -> String {
    chat_markdown(source, "• ")
}

pub fn synology_markdown(source: &str) -> String {
    chat_markdown(source, "* ")
}

fn chat_markdown(source: &str, list_marker: &str) -> String {
    fn escape(value: &str) -> String {
        value
            .replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
    }
    let mut result = String::new();
    let mut links = Vec::new();
    for event in Parser::new_ext(source, Options::ENABLE_STRIKETHROUGH) {
        match event {
            MarkdownEvent::Text(value) => result.push_str(&escape(&value)),
            MarkdownEvent::Code(value) => result.push_str(&format!("`{}`", escape(&value))),
            MarkdownEvent::Html(value) | MarkdownEvent::InlineHtml(value) => {
                result.push_str(&escape(&value))
            }
            MarkdownEvent::SoftBreak | MarkdownEvent::HardBreak => result.push('\n'),
            MarkdownEvent::Start(tag) => match tag {
                Tag::Strong | Tag::Heading { .. } => result.push('*'),
                Tag::Emphasis => result.push('_'),
                Tag::Strikethrough => result.push('~'),
                Tag::CodeBlock(_) => result.push_str("```\n"),
                Tag::Item => result.push_str(list_marker),
                Tag::Link { dest_url, .. } | Tag::Image { dest_url, .. } => {
                    let valid = !links.iter().any(|active| *active)
                        && reqwest::Url::parse(&dest_url)
                            .is_ok_and(|url| ["http", "https", "mailto"].contains(&url.scheme()));
                    links.push(valid);
                    if valid {
                        result.push('<');
                        result.push_str(&escape(&dest_url.replace('|', "%7C")));
                        result.push('|');
                    }
                }
                _ => {}
            },
            MarkdownEvent::End(tag) => match tag {
                TagEnd::Strong => result.push('*'),
                TagEnd::Heading(_) => result.push_str("*\n"),
                TagEnd::Emphasis => result.push('_'),
                TagEnd::Strikethrough => result.push('~'),
                TagEnd::CodeBlock => result.push_str("\n```\n"),
                TagEnd::Link | TagEnd::Image => {
                    if links.pop() == Some(true) {
                        result.push('>');
                    }
                }
                TagEnd::Paragraph | TagEnd::Item => result.push('\n'),
                _ => {}
            },
            MarkdownEvent::Rule => result.push_str("\n────\n"),
            _ => {}
        }
    }
    result.trim().to_owned()
}
