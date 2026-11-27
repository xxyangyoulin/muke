use serde_json::{Value, json};
use std::sync::LazyLock;

static URL: LazyLock<regex::Regex> =
    LazyLock::new(|| regex::Regex::new(r"(?iu)\bhttps?://[^\s<]+\b/*").unwrap());

pub fn article(article: &Value) -> String {
    let source = article["content"].as_str().unwrap_or("");
    if article["format"] == "html" {
        return clean(source, false);
    }
    use pulldown_cmark::{Event, LinkType, Options, Parser, Tag, TagEnd, html};
    let parser = Parser::new_ext(
        source,
        Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH,
    );
    // 先按源码区间合并相邻文本事件（pulldown 会把疑似定界符的下划线拆成独立事件），
    // 再做 文本格式处理：保留其转义集之外的反斜杠、允许词内 `_` 强调（闭合端
    // 只要求词边界）、裸 URL 转链接。
    let mut events = Vec::new();
    let mut literal_depth = 0usize;
    let mut run = String::new();
    let mut escaped = Vec::new();
    let flush =
        |run: &mut String, escaped: &mut Vec<bool>, events: &mut Vec<Event>, depth: usize| {
            if run.is_empty() {
                return;
            }
            if depth > 0 {
                events.push(Event::Text(run.clone().into()));
            } else {
                events.extend(split_underscore(run, escaped));
            }
            run.clear();
            escaped.clear();
        };
    for (event, range) in parser.into_offset_iter() {
        match event {
            Event::Text(text) => {
                let restore_escape = source.as_bytes()[..range.start]
                    .iter()
                    .rev()
                    .take_while(|&&byte| byte == b'\\')
                    .count()
                    % 2
                    == 1;
                if restore_escape
                    && text.chars().next().is_some_and(|c| {
                        c.is_ascii_punctuation() && !r"\`*_{}[]()>#+-.!|".contains(c)
                    })
                {
                    run.push('\\');
                    escaped.push(false);
                }
                let escaped_underscore =
                    source.as_bytes()[range.start] == b'\\' && text.starts_with('_');
                for c in text.chars() {
                    run.push(c);
                    for _ in 0..c.len_utf8() {
                        escaped.push(escaped_underscore);
                    }
                }
            }
            Event::Start(Tag::CodeBlock(_) | Tag::Image { .. }) => {
                flush(&mut run, &mut escaped, &mut events, literal_depth);
                literal_depth += 1;
                events.push(event);
            }
            Event::End(TagEnd::CodeBlock | TagEnd::Image) => {
                flush(&mut run, &mut escaped, &mut events, literal_depth);
                literal_depth = literal_depth.saturating_sub(1);
                events.push(event);
            }
            Event::SoftBreak => {
                flush(&mut run, &mut escaped, &mut events, literal_depth);
                events.push(Event::HardBreak);
            }
            Event::Html(_) | Event::InlineHtml(_) => {
                flush(&mut run, &mut escaped, &mut events, literal_depth);
                if let Event::Html(text) | Event::InlineHtml(text) = event {
                    events.push(Event::Text(text));
                }
            }
            event => {
                flush(&mut run, &mut escaped, &mut events, literal_depth);
                events.push(event);
            }
        }
    }
    flush(&mut run, &mut escaped, &mut events, literal_depth);
    let mut linked = Vec::new();
    let mut literal_depth = 0;
    for event in events {
        match event {
            Event::Start(Tag::CodeBlock(_) | Tag::Link { .. } | Tag::Image { .. }) => {
                literal_depth += 1;
                linked.push(event);
            }
            Event::End(TagEnd::CodeBlock | TagEnd::Link | TagEnd::Image) => {
                literal_depth -= 1;
                linked.push(event);
            }
            Event::Text(text) if literal_depth == 0 => {
                let mut offset = 0;
                for matched in URL.find_iter(&text) {
                    if matched.start() > offset {
                        linked.push(Event::Text(text[offset..matched.start()].to_owned().into()));
                    }
                    linked.push(Event::Start(Tag::Link {
                        link_type: LinkType::Autolink,
                        dest_url: matched.as_str().to_owned().into(),
                        title: "".into(),
                        id: "".into(),
                    }));
                    linked.push(Event::Text(matched.as_str().to_owned().into()));
                    linked.push(Event::End(TagEnd::Link));
                    offset = matched.end();
                }
                if offset == 0 {
                    linked.push(Event::Text(text));
                } else if offset < text.len() {
                    linked.push(Event::Text(text[offset..].to_owned().into()));
                }
            }
            event => linked.push(event),
        }
    }
    let mut output = String::new();
    html::push_html(&mut output, linked.into_iter());
    clean(
        &output
            .replace("\n</code></pre>", "</code></pre>")
            .replace("</th>", "</th>\n")
            .replace("</td>", "</td>\n"),
        true,
    )
}

/// 按词内强调规则在一段文本内拆出 `_`/`__` 强调（闭合端才要求词边界）。
/// 下划线出现在 URL 匹配范围内时跳过，交给后续裸链接处理。
fn split_underscore(text: &str, escaped: &[bool]) -> Vec<pulldown_cmark::Event<'static>> {
    use pulldown_cmark::{Event, Tag, TagEnd};
    let bytes = text.as_bytes();
    let urls: Vec<(usize, usize)> = URL
        .find_iter(text)
        .map(|matched| (matched.start(), matched.end()))
        .collect();
    let in_url = |position: usize| {
        urls.iter()
            .any(|(start, end)| position >= *start && position < *end)
    };
    let mut events = Vec::new();
    let mut plain = 0;
    let mut index = 0;
    while index < text.len() {
        let step = text[index..].chars().next().map_or(1, char::len_utf8);
        if bytes[index] != b'_' || in_url(index) || escaped.get(index).copied().unwrap_or(false) {
            index += step;
            continue;
        }
        let Some((content_end, after)) = underscore_close(text, index) else {
            index += step;
            continue;
        };
        if plain < index {
            events.push(Event::Text(text[plain..index].to_owned().into()));
        }
        let strong = bytes[index + 1] == b'_';
        if strong {
            events.push(Event::Start(Tag::Strong));
        } else {
            events.push(Event::Start(Tag::Emphasis));
        }
        events.push(Event::Text(
            text[index + if strong { 2 } else { 1 }..content_end]
                .to_owned()
                .into(),
        ));
        events.push(if strong {
            Event::End(TagEnd::Strong)
        } else {
            Event::End(TagEnd::Emphasis)
        });
        index = after;
        plain = index;
    }
    if events.is_empty() {
        return vec![Event::Text(text.to_owned().into())];
    }
    if plain < text.len() {
        events.push(Event::Text(text[plain..].to_owned().into()));
    }
    events
}

/// 从 open 起按 `_`（Em）/`__`（Strong）正则找闭合，返回 (内容结束, 整体结束)。
fn underscore_close(text: &str, open: usize) -> Option<(usize, usize)> {
    let bytes = text.as_bytes();
    let strong = bytes.get(open + 1) == Some(&b'_');
    let mut index = open + if strong { 2 } else { 1 };
    while index < text.len() {
        match bytes[index] {
            b'\\' if bytes.get(index + 1) == Some(&b'_') => index += 2,
            b'_' => {
                let doubled = bytes.get(index + 1) == Some(&b'_');
                match (strong, doubled) {
                    (true, true) => {
                        // 闭合候选 `__`（后面不能再跟 `_`），或内容里的 `__x__` 对
                        if bytes.get(index + 2) != Some(&b'_') {
                            return finish(text, open, index, 2);
                        }
                        let mut scan = index + 2;
                        while scan < text.len() && bytes[scan] != b'_' {
                            scan += 1;
                        }
                        if bytes.get(scan + 1) == Some(&b'_') {
                            index = scan + 2;
                        } else {
                            return None;
                        }
                    }
                    (true, false) => {
                        // 强调内容允许 `_x_` 对
                        let mut scan = index + 1;
                        while scan < text.len() && bytes[scan] != b'_' {
                            scan += 1;
                        }
                        if scan < text.len() && scan > index + 1 {
                            index = scan + 1;
                        } else {
                            return None;
                        }
                    }
                    (false, true) => {
                        // 斜体内容允许 `__x__` 对
                        let mut scan = index + 2;
                        while scan < text.len() && bytes[scan] != b'_' {
                            scan += 1;
                        }
                        if bytes.get(scan + 1) == Some(&b'_') {
                            index = scan + 2;
                        } else {
                            return None;
                        }
                    }
                    (false, false) => {
                        // 闭合候选 `_`：后面不能是 `_`，且须有词边界
                        if !word_boundary_after(text, index + 1) {
                            return None;
                        }
                        return finish(text, open, index, 1);
                    }
                }
            }
            _ => index += text[index..].chars().next().map_or(1, char::len_utf8),
        }
    }
    None
}

fn finish(text: &str, open: usize, close: usize, width: usize) -> Option<(usize, usize)> {
    let content_start = open + if width == 2 { 2 } else { 1 };
    if close <= content_start {
        return None;
    }
    Some((close, (close + width).min(text.len())))
}

fn word_boundary_after(text: &str, position: usize) -> bool {
    text[position..]
        .chars()
        .next()
        .is_none_or(|next| !next.is_alphanumeric() && next != '_')
}

fn clean(source: &str, markdown: bool) -> String {
    let mut builder = ammonia::Builder::default();
    builder
        .add_tag_attributes("th", ["style"])
        .add_tag_attributes("td", ["style"])
        .filter_style_properties(["text-align"].into_iter().collect());
    if markdown {
        builder.add_tag_attributes("code", ["class"]);
    }
    builder.clean(source).to_string()
}

pub fn summarize(item: &mut Value) {
    let html = article(item);
    let document = scraper::Html::parse_fragment(&html);
    let plain = document.root_element().text().collect::<String>();
    let plain = plain.split_whitespace().collect::<Vec<_>>().join(" ");
    let length = plain.chars().count();
    item["excerpt"] =
        json!(plain.chars().take(110).collect::<String>() + if length > 110 { "…" } else { "" });
    item["text_length"] = json!(length);
    let selector = scraper::Selector::parse("img[src]").unwrap();
    item["cover"] = document
        .select(&selector)
        .next()
        .and_then(|e| e.value().attr("src"))
        .filter(|src| {
            let lower = src.to_ascii_lowercase();
            lower.starts_with("http://")
                || lower.starts_with("https://")
                || (src.starts_with('/') && !src.starts_with("//"))
        })
        .map(|src| {
            if let Some(name) = src
                .strip_prefix("/image?upload=")
                .map(|value| value.split('&').next().unwrap())
                .filter(|name| crate::api::upload_name(name))
            {
                json!(format!("/image?upload={name}&thumb=1"))
            } else {
                json!(src)
            }
        })
        .unwrap_or(Value::Null);
    item.as_object_mut().unwrap().remove("content");
    item.as_object_mut().unwrap().remove("images");
}
