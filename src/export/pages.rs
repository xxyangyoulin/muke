use super::data::Data;
use crate::plugins;
use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use std::{path::Path, sync::LazyLock};
use tokio_util::sync::CancellationToken;

pub fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}
pub(crate) fn text(value: &Value, key: &str) -> String {
    escape(value[key].as_str().unwrap_or(""))
}
pub(crate) fn external_url(value: &str) -> Option<String> {
    let url = url::Url::parse(value).ok()?;
    matches!(url.scheme(), "http" | "https").then(|| escape(url.as_str()))
}
fn json_script(value: &Value, id: &str) -> Result<String> {
    Ok(format!(
        "<script type=\"application/json\" id=\"{id}\">{}</script>",
        serde_json::to_string(value)?.replace('<', "\\u003c")
    ))
}
fn absolute(site: &str, path: &str) -> Option<String> {
    let base = url::Url::parse(site).ok()?;
    if !matches!(base.scheme(), "http" | "https") {
        return None;
    }
    base.join(path).ok().map(|url| url.to_string())
}

pub fn write(
    frontend: &Path,
    destination: &Path,
    data: &Data,
    stop: &CancellationToken,
) -> Result<()> {
    let mut shell = std::fs::read_to_string(frontend.join("index.html"))?;
    let state = &data.content["state"];
    let profile = &state["profile"];
    let articles = data.articles["articles"]
        .as_array()
        .context("文章静态数据无效")?;
    let posts = data.content["posts"]
        .as_array()
        .context("动态静态数据无效")?;
    let mut pages = vec![("/".to_owned(), None), ("/articles/".to_owned(), None)];
    let mut plugin_pages = std::collections::BTreeMap::new();
    let mut styles = String::new();
    let mut navigation = String::new();
    let mut first_plugin_route = None;
    if let Some(items) = state["plugins"].as_object() {
        for (id, item) in items {
            if let Some(style) = item["style"].as_str() {
                styles.push_str(&format!(
                    r#"<link rel="stylesheet" href="{}">"#,
                    escape(style)
                ));
            }
            if let Some(route) = item["page"]["route"].as_str() {
                let route = format!("{}/", route.trim_end_matches('/'));
                if first_plugin_route.is_none() {
                    first_plugin_route = Some(route.clone());
                }
                // 与 PHP 端一致：声明了 page.nav 的插件进左侧导航（图标/标签/计数徽标），
                // 仅 page.tab 的插件（如 RSS）走首页顶栏标签
                if let Some(nav) = item["page"]["nav"].as_object() {
                    let label = nav
                        .get("label")
                        .and_then(Value::as_str)
                        .or_else(|| item["page"]["title"].as_str())
                        .unwrap_or("");
                    let count = if nav.get("count") == Some(&Value::Bool(true)) {
                        format!(
                            r#"<span class="nav-count" data-plugin-count="{}">—</span>"#,
                            escape(id)
                        )
                    } else {
                        String::new()
                    };
                    navigation.push_str(&format!(
                        r#"<a href="{}" data-nav="{}" title="{}"><span data-icon="{}"></span><span class="nav-text">{}</span>{}</a>"#,
                        escape(&route),
                        escape(id),
                        escape(label),
                        escape(nav.get("icon").and_then(Value::as_str).unwrap_or("grid")),
                        escape(label),
                        count
                    ));
                }
                pages.push((route.clone(), None));
                plugin_pages.insert(route, (id, &item["page"]));
            }
        }
    }
    shell = shell.replacen("</nav>", &format!("{navigation}</nav>"), 1);
    pages.extend(
        articles
            .iter()
            .map(|article| (format!("/article/{}/", article["id"]), Some(article))),
    );
    let embedded = json_script(&data.content, "embedded-content")?
        + &json_script(&data.articles, "embedded-articles")?;
    for (route, article) in pages {
        if stop.is_cancelled() {
            bail!("任务已取消");
        }
        let plugin_page = plugin_pages.get(&route);
        let title = if let Some((_, page)) = plugin_page {
            format!("{} · {}", text(page, "title"), text(profile, "name"))
        } else if let Some(article) = article {
            format!("{} · {}", text(article, "title"), text(profile, "name"))
        } else if route == "/articles/" {
            format!("文章随笔 · {}", text(profile, "name"))
        } else {
            format!(
                "{} · {}",
                text(profile, "name"),
                text(&state["options"], "site_tagline")
            )
        };
        let description = article
            .map(|article| text(article, "excerpt"))
            .or_else(|| plugin_page.map(|(_, page)| text(page, "description")))
            .unwrap_or_else(|| text(&state["options"], "site_description"));
        // 打包后的产物只有一个经典脚本；回退为模块化时保留 registry + module 两个标签
        let scripts = if destination.join("assets/registry.js").exists() {
            r#"<script src="/assets/registry.js"></script><script type="module" src="/assets/app.js"></script>"#
        } else {
            r#"<script src="/assets/app.js"></script>"#
        };
        let mut head = format!(
            r##"<meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1"><meta name="theme-color" content="#f6f7f9"><title>{title}</title><meta name="description" content="{description}"><meta property="og:title" content="{title}"><meta property="og:description" content="{description}"><meta property="og:type" content="{}"><meta property="og:site_name" content="{}"><meta name="twitter:card" content="summary"><meta name="twitter:title" content="{title}"><meta name="twitter:description" content="{description}"><link rel="alternate" type="application/rss+xml" href="/rss.xml"><link rel="icon" href="/assets/favicon.svg" type="image/svg+xml"><link rel="stylesheet" href="/assets/style.css">{scripts}"##,
            if article.is_some() {
                "article"
            } else {
                "website"
            },
            text(profile, "name")
        );
        head.push_str(&styles);
        let canonical = absolute(&data.site_url, &route);
        if let Some(url) = &canonical {
            head.push_str(&format!(
                r#"<link rel="canonical" href="{}"><meta property="og:url" content="{}">"#,
                escape(url),
                escape(url)
            ));
        }
        let mut body = String::new();
        if let Some((id, _)) = plugin_page {
            body = plugins::get(id)?
                .export_page(&data.content["plugins"][*id])
                .with_context(|| format!("插件 {id} 未提供静态页面"))?;
        } else if let Some(article) = article {
            body.push_str(&format!(r#"<article class="article-page"><h1>{}</h1><time>{}</time><div class="article-content">{}</div></article>"#,text(article,"title"),text(article,"created_at"),article["html"].as_str().unwrap_or("")));
            body.push_str(&comments(&article["comments"]));
            let mut schema = json!({"@context":"https://schema.org","@type":"BlogPosting","headline":article["title"],"description":article["excerpt"],"datePublished":article["created_at"],"dateModified":article["updated_at"],"author":{"@type":"Person","name":profile["name"]}});
            if let Some(url) = canonical {
                schema["mainEntityOfPage"] = json!(url);
            }
            if let Some(image) = article["cover"]
                .as_str()
                .and_then(|path| absolute(&data.site_url, path))
            {
                head.push_str(&format!(
                    r#"<meta property="og:image" content="{}">"#,
                    escape(&image)
                ));
                schema["image"] = json!(image);
            }
            head.push_str(&format!(
                r#"<script type="application/ld+json">{}</script>"#,
                serde_json::to_string(&schema)?.replace('<', "\\u003c")
            ));
        } else {
            if route == "/" {
                body.push_str(&format!(
                    "<section class=\"feed-profile\"><h1>{}</h1><p>{}</p></section>",
                    text(profile, "name"),
                    text(profile, "bio")
                ));
                for post in posts {
                    body.push_str(&format!(
                        r#"<article class="post" id="post-{}"><div class="post-body">"#,
                        post["id"]
                    ));
                    let rendered = post["source"]
                        .as_str()
                        .and_then(|id| plugins::get(id).ok())
                        .and_then(|plugin| plugin.render_post(post));
                    if let Some(rendered) = rendered {
                        body.push_str(&rendered);
                    } else {
                        body.push_str(&format!(
                            r#"<p class="post-content">{}</p>"#,
                            text(post, "content")
                        ));
                        if let Some(images) = post["images"].as_array() {
                            for image in images {
                                body.push_str(&format!(
                                    r#"<a href="{}"><img src="{}" alt="" loading="lazy"></a>"#,
                                    text(image, "url"),
                                    text(image, "thumb")
                                ));
                            }
                        }
                    }
                    if let Some(link) = post["link"].as_str().and_then(external_url) {
                        body.push_str(&format!(
                            r#"<a href="{link}" target="_blank" rel="noopener">原文</a>"#
                        ));
                    }
                    body.push_str(&format!(
                        "<time>{}</time></div></article>",
                        text(post, "created_at")
                    ));
                    body.push_str(&comments(&post["comments"]));
                }
            }
            body.push_str("<section class=\"articles-page\"><h1>文章随笔</h1>");
            for article in articles {
                body.push_str(&format!(
                    r#"<a class="article-card" href="/article/{}/"><h2>{}</h2><p>{}</p></a>"#,
                    article["id"],
                    text(article, "title"),
                    text(article, "excerpt")
                ));
            }
            body.push_str("</section>");
        }
        let html = page_shell(
            &shell,
            &head,
            &embedded,
            &body,
            profile,
            first_plugin_route.as_deref().unwrap_or("/articles/"),
        )?;
        let path = destination
            .join(route.trim_start_matches('/'))
            .join("index.html");
        std::fs::create_dir_all(path.parent().unwrap())?;
        std::fs::write(path, html)?;
    }
    Ok(())
}

fn comments(items: &Value) -> String {
    let Some(items) = items.as_array().filter(|items| !items.is_empty()) else {
        return String::new();
    };
    let mut html = String::from("<section class=\"comments-panel\"><h2>评论</h2>");
    for item in items {
        html.push_str(&format!(
            "<article class=\"comment-row\"><strong>{}</strong><time>{}</time><p>{}</p></article>",
            text(item, "author"),
            text(item, "created_at"),
            text(item, "content")
        ));
    }
    html.push_str("</section>");
    html
}

fn page_shell(
    shell: &str,
    head: &str,
    embedded: &str,
    body: &str,
    profile: &Value,
    explore: &str,
) -> Result<String> {
    static ADMIN: LazyLock<regex::Regex> = LazyLock::new(|| {
        regex::Regex::new(r#"(?s)<a\b[^>]*href="/admin"[^>]*>.*?</a>|<button\b[^>]*id="new-post"[^>]*>.*?</button>"#).unwrap()
    });
    let mut html = ADMIN
        .replace_all(shell, "")
        .into_owned()
        .replace("木刻", &text(profile, "name"));
    // 与 PHP 静态页一致：访客的「探索风景」入口指向第一个插件页（无则文章列表）
    let explore = format!(
        r#"<a class="primary publish-button" id="new-post" href="{}" title="探索风景"><span data-icon="image"></span><span class="nav-text">探索风景</span></a>"#,
        escape(explore)
    );
    html = html.replacen(
        r#"<div class="sidebar-bottom">"#,
        &format!(r#"{explore}<div class="sidebar-bottom">"#),
        1,
    );
    let start = html.find("<head>").context("前端页面缺少 head")? + 6;
    let end = html.find("</head>").context("前端页面缺少 head 结束标记")?;
    html.replace_range(start..end, head);
    html = html.replacen("<html ", "<html data-static=\"true\" ", 1);
    let loading = "<div class=\"loading\">正在载入…</div>";
    if !html.contains(loading) {
        bail!("前端页面缺少内容入口");
    }
    html = html.replacen(
        loading,
        &format!("{loading}{embedded}<div id=\"baked-body\">{body}</div>"),
        1,
    );
    Ok(html)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn core_pages_bake_content_and_embed_safe_json() {
        let root = tempfile::tempdir().unwrap();
        let title = "标题 </script><img id='injected'>";
        let data = Data {
            site_url: "https://example.org".into(),
            content: json!({"state":{"profile":{"name":"测试 <站点>","bio":"介绍"},"options":{"site_tagline":"记录","site_description":"描述"}},"posts":[{"id":1,"content":"动态 <script>bad()</script>","created_at":"2026-11-26","images":[]}],"plugins":{},"plugins_meta":{}}),
            articles: json!({"articles":[{"id":7,"title":title,"html":"<p>文章正文</p><a href=\"/admin\">正文中的管理链接</a>","excerpt":"摘要","created_at":"2026-11-26","updated_at":"2026-11-26","comments":[{"author":"读者","content":"留言 <b>text</b>","created_at":"2026"}]}]}),
        };
        let frontend = Path::new(env!("CARGO_MANIFEST_DIR")).join("frontend");
        write(&frontend, root.path(), &data, &CancellationToken::new()).unwrap();
        for file in ["index.html", "articles/index.html", "article/7/index.html"] {
            let source = std::fs::read_to_string(root.path().join(file)).unwrap();
            let html = scraper::Html::parse_document(&source);
            let selector = |value| scraper::Selector::parse(value).unwrap();
            assert_eq!(
                html.root_element().value().attr("data-static"),
                Some("true")
            );
            assert!(
                html.select(&selector("nav [data-nav=admin], #injected"))
                    .next()
                    .is_none()
            );
            // 静态访客入口「探索风景」复用 #new-post（与 PHP 一致）
            let explore = html.select(&selector("#new-post")).next().unwrap();
            let link = explore.value().attr("href").unwrap_or_default();
            assert!(link.starts_with('/'), "探索风景入口：{link}");
            assert!(
                html.select(&selector(
                    "#edit-form, #confirm-delete, #confirm-plugin-remove"
                ))
                .count()
                    == 3
            );
            for (id, expected) in [
                ("embedded-content", &data.content),
                ("embedded-articles", &data.articles),
            ] {
                let query = scraper::Selector::parse(&format!("#{id}")).unwrap();
                let element = html.select(&query).next().unwrap();
                let decoded: Value =
                    serde_json::from_str(&element.text().collect::<String>()).unwrap();
                assert_eq!(&decoded, expected);
            }
            assert!(html.select(&selector("#baked-body")).next().is_some());
        }
        let article = scraper::Html::parse_document(
            &std::fs::read_to_string(root.path().join("article/7/index.html")).unwrap(),
        );
        let selector = scraper::Selector::parse("h1").unwrap();
        assert_eq!(
            article
                .select(&selector)
                .next()
                .unwrap()
                .text()
                .collect::<String>(),
            title
        );
        let selector = scraper::Selector::parse("link[rel=canonical]").unwrap();
        assert_eq!(
            article
                .select(&selector)
                .next()
                .unwrap()
                .value()
                .attr("href"),
            Some("https://example.org/article/7/")
        );
        let selector = scraper::Selector::parse(".article-content a").unwrap();
        assert_eq!(
            article
                .select(&selector)
                .next()
                .unwrap()
                .value()
                .attr("href"),
            Some("/admin")
        );
        let selector = scraper::Selector::parse("script[type='application/ld+json']").unwrap();
        let schema: Value = serde_json::from_str(
            &article
                .select(&selector)
                .next()
                .unwrap()
                .text()
                .collect::<String>(),
        )
        .unwrap();
        assert_eq!(schema["headline"], title);
        let cancelled = CancellationToken::new();
        cancelled.cancel();
        assert!(write(&frontend, &root.path().join("cancelled"), &data, &cancelled).is_err());
        assert!(!root.path().join("cancelled").exists());
    }
}
