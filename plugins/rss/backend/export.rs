use crate::export::pages::{external_url, text};
use serde_json::Value;

pub fn post(post: &Value) -> String {
    let meta = &post["meta"];
    let title = text(meta, "title");
    let title = if let Some(link) = post["link"].as_str().and_then(external_url) {
        format!(r#"<a class="rss-title" href="{link}" target="_blank" rel="noopener">{title}</a>"#)
    } else {
        format!(r#"<strong class="rss-title">{title}</strong>"#)
    };
    let mut html = format!(
        r#"<div class="post-rss-card"><p class="rss-line"><span class="rss-feed-tag">{}</span>{title}</p><p class="rss-summary">{}</p>"#,
        text(meta, "name"),
        text(meta, "summary")
    );
    if let Some(cover) = post["images"].as_array().and_then(|images| images.first()) {
        html.push_str(&format!(
            r#"<img class="rss-cover" src="{}" alt="" loading="lazy">"#,
            text(cover, "thumb")
        ));
    }
    html.push_str("</div>");
    html
}

pub fn page(data: &Value) -> String {
    let posts = data["posts"].as_array().unwrap();
    let feeds = data["feeds"].as_array().unwrap();
    let mut html = format!(
        r#"<section class="rss-page"><div class="rss-hero"><h1>RSS 订阅。</h1><p>聚合我订阅的站点更新，点击标题可去原文。</p><div class="rss-facts"><span><strong>{}</strong> 个源</span><span><strong>{}</strong> 条动态</span></div></div><div class="rss-static-list">"#,
        feeds.len(),
        posts.len()
    );
    for item in posts {
        html.push_str(&format!(
            r#"<article class="post rss-static-item" id="post-{}">"#,
            item["id"]
        ));
        if item["meta"]["avatar"].is_object() {
            html.push_str(&format!(
                r#"<img class="avatar" src="{}" alt="{}">"#,
                text(&item["meta"]["avatar"], "thumb"),
                text(&item["meta"], "name")
            ));
        }
        html.push_str(&format!(
            r#"<div class="post-body">{}<div class="rss-static-meta"><time>{}</time>"#,
            post(item),
            text(item, "created_at")
        ));
        if let Some(link) = item["link"].as_str().and_then(external_url) {
            html.push_str(&format!(
                r#"<a href="{link}" target="_blank" rel="noopener">原文</a>"#
            ));
        }
        html.push_str("</div></div></article>");
    }
    if posts.is_empty() {
        html.push_str(r#"<div class="empty-state"><h3>还没有订阅内容</h3></div>"#);
    }
    html.push_str("</div></section>");
    html
}
