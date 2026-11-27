use super::{Plugin, import};
use crate::{api::App, config::Settings, error::Result, network, tasks};
use anyhow::bail;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use roxmltree::Node;
use serde::Deserialize;
use serde_json::{Value, json};
use sha1::{Digest, Sha1};
use std::sync::LazyLock;

pub struct Weibo;
static MANIFEST: LazyLock<Value> =
    LazyLock::new(|| serde_json::from_str(include_str!("manifest.json")).unwrap());
static STATUS: LazyLock<regex::Regex> =
    LazyLock::new(|| regex::Regex::new(r"/status/(\d+)").unwrap());

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct Sync {
    all: bool,
}
struct Entry {
    key: String,
    link: String,
    text: String,
    author: String,
    timestamp: i64,
    images: Vec<String>,
}
fn child<'a, 'input>(node: Node<'a, 'input>, name: &str) -> Option<Node<'a, 'input>> {
    node.children()
        .find(|n| n.is_element() && n.tag_name().name() == name)
}
fn text(node: Node<'_, '_>, name: &str) -> String {
    child(node, name)
        .and_then(|n| n.text())
        .unwrap_or("")
        .trim()
        .into()
}
fn parse(xml: &[u8], timezone: chrono_tz::Tz) -> anyhow::Result<Vec<Entry>> {
    let xml = super::feed::decode(xml)?;
    let doc = roxmltree::Document::parse(&xml)?;
    let root = doc.root_element();
    let container = child(root, "channel").unwrap_or(root);
    if !matches!(root.tag_name().name(), "rss" | "feed") {
        bail!("不是有效的 RSS/Atom 订阅源");
    }
    let mut entries = Vec::new();
    for node in container
        .children()
        .filter(|n| n.is_element() && matches!(n.tag_name().name(), "item" | "entry"))
    {
        let link = node
            .children()
            .filter(|n| n.is_element() && n.tag_name().name() == "link")
            .find_map(|n| {
                n.attribute("rel")
                    .is_none_or(|r| r == "alternate")
                    .then(|| n.attribute("href").or(n.text()))
                    .flatten()
            })
            .unwrap_or("")
            .trim()
            .to_owned();
        if network::parse_url(&link).is_err() {
            continue;
        }
        let key = STATUS
            .captures(&link)
            .map(|m| m[1].to_owned())
            .unwrap_or_else(|| hex::encode(Sha1::digest(link.as_bytes()))[..12].into());
        let body_node = ["encoded", "content", "summary", "description"]
            .iter()
            .filter_map(|name| child(node, name))
            .find(|n| {
                n.descendants()
                    .any(|part| part.is_text() && !part.text().unwrap_or("").trim().is_empty())
                    || n.attribute("type") == Some("xhtml")
            });
        let xhtml = body_node.filter(|n| n.attribute("type") == Some("xhtml"));
        let html = body_node.and_then(|n| n.text()).unwrap_or("").trim();
        let document = scraper::Html::parse_fragment(html);
        let body = if let Some(body) = xhtml {
            body.descendants()
                .filter(|n| n.is_text())
                .filter_map(|n| n.text())
                .collect::<String>()
        } else {
            document.root_element().text().collect::<String>()
        };
        let body = body.split_whitespace().collect::<Vec<_>>().join(" ");
        let body = if body.is_empty() {
            super::feed::title(node)
        } else {
            body
        };
        if body.is_empty() {
            continue;
        }
        let images = if let Some(body) = xhtml {
            body.descendants()
                .filter(|n| n.is_element() && n.tag_name().name() == "img")
                .filter_map(|n| n.attribute("src"))
                .take(4)
                .map(str::to_owned)
                .collect()
        } else {
            document
                .select(&scraper::Selector::parse("img[src]").unwrap())
                .take(4)
                .map(|n| n.value().attr("src").unwrap().to_owned())
                .collect()
        };
        let date = ["pubDate", "updated", "published"]
            .iter()
            .map(|name| text(node, name))
            .find(|s| !s.is_empty())
            .unwrap_or_default();
        let timestamp =
            super::feed::timestamp(&date, timezone).unwrap_or_else(|| Utc::now().timestamp());
        entries.push(Entry {
            key: format!("weibo:{key}"),
            link,
            text: body,
            author: child(node, "author")
                .map(|n| text(n, "name"))
                .unwrap_or_default(),
            timestamp,
            images,
        });
    }
    Ok(entries)
}

#[async_trait]
impl Plugin for Weibo {
    fn render_post(&self, post: &Value) -> Option<String> {
        use crate::export::pages::{escape, text};
        let content = post["meta"]["text"]
            .as_str()
            .or_else(|| post["content"].as_str())
            .unwrap_or("");
        let mut html = format!(
            r#"<div class="post-weibo-card"><p class="weibo-text"><span class="weibo-brand">微博</span>{}</p>"#,
            escape(content)
        );
        if let Some(images) = post["images"].as_array() {
            html.push_str(&format!(
                r#"<div class="weibo-grid count{}">"#,
                images.len().min(3)
            ));
            for image in images.iter().take(3) {
                html.push_str(&format!(
                    r#"<a class="post-image" href="{}"><img src="{}" alt="" loading="lazy"></a>"#,
                    text(image, "url"),
                    text(image, "thumb")
                ));
            }
            html.push_str("</div>");
        }
        html.push_str("</div>");
        Some(html)
    }
    fn id(&self) -> &'static str {
        "weibo"
    }
    fn manifest(&self) -> &'static Value {
        &MANIFEST
    }
    async fn config_changed(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
        config: &Settings,
    ) -> Result<()> {
        let rows: Vec<(i64, String)> =
            sqlx::query_as("SELECT id,meta FROM posts WHERE source='weibo'")
                .fetch_all(&mut **tx)
                .await?;
        for (id, raw) in rows {
            let mut meta = serde_json::from_str::<Value>(&raw)
                .ok()
                .filter(Value::is_object)
                .unwrap_or(json!({}));
            meta["hide_source"] = json!(if config.enabled("show_source") { 0 } else { 1 });
            sqlx::query("UPDATE posts SET meta=? WHERE id=?")
                .bind(meta.to_string())
                .bind(id)
                .execute(&mut **tx)
                .await?;
        }
        Ok(())
    }
    async fn sync(&self, app: &App, ctx: &tasks::Context, params: Value) -> anyhow::Result<Value> {
        let options: Sync = serde_json::from_value(params)?;
        let config = super::config(self, &Settings::load(app).await?);
        let xml = app
            .network
            .get(config.get("feed_url"), true, 8 * 1024 * 1024, &ctx.stop)
            .await?;
        let entries = parse(&xml, app.config.timezone)?;
        let limit = if options.all {
            50
        } else {
            config.number("max_items") as usize
        };
        let mut added = 0;
        for entry in entries.into_iter().take(limit) {
            ctx.check()?;
            if import::known(app, self.id(), &entry.key).await? {
                continue;
            }
            let mut images = Vec::new();
            for url in &entry.images {
                ctx.check()?;
                if let Ok(name) =
                    import::fetch_image(app, ctx, url, 120, "https://weibo.com/").await
                {
                    images.push(name);
                }
            }
            let author = if !config.get("author").is_empty() {
                config.get("author")
            } else if !entry.author.is_empty() {
                &entry.author
            } else {
                "微博"
            };
            let meta = json!({"card":"weibo", "name":author,"text":entry.text.chars().take(300).collect::<String>(),"hide_source":if config.enabled("show_source") {0} else {1}});
            let created = DateTime::from_timestamp(entry.timestamp, 0)
                .unwrap_or_else(Utc::now)
                .with_timezone(&app.config.timezone)
                .to_rfc3339();
            added += import::save(
                app,
                ctx,
                self.id(),
                import::Post {
                    key: entry.key,
                    content: entry.text.chars().take(500).collect(),
                    created_at: created,
                    link: entry.link,
                    meta,
                    images,
                },
            )
            .await?;
            ctx.progress("同步微博", added as i64, None).await?;
        }
        ctx.check()?;
        Ok(json!({"added":added}))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn atom_title_fallback_and_local_date() {
        let xml = br#"<feed xmlns="http://www.w3.org/2005/Atom"><title type="html">&lt;b&gt;Feed&lt;/b&gt;</title><entry><id>one</id><title type="xhtml"><div xmlns="http://www.w3.org/1999/xhtml">A <b>title</b> &amp; &lt;literal&gt;</div></title><link href="https://weibo.com/status/123"/><updated>2000-01-01 08:00:00</updated></entry></feed>"#;
        let entries = parse(xml, chrono_tz::Asia::Shanghai).unwrap();
        assert_eq!(entries[0].text, "A title & <literal>");
        assert_eq!(entries[0].timestamp, 946684800);
    }
    #[test]
    fn declared_gbk_and_utf16_feeds() {
        let xml = "<?xml version='1.0' encoding='GBK'?><rss><channel><title>订阅</title><item><guid>one</guid><title>标题</title><link>https://weibo.com/status/123</link><description>中文正文</description></item></channel></rss>";
        let (bytes, _, errors) = encoding_rs::GBK.encode(xml);
        assert!(!errors);
        let entries = parse(&bytes, chrono_tz::Asia::Shanghai).unwrap();
        assert_eq!(entries[0].text, "中文正文");
        let xml = xml.replace("GBK", "UTF-16");
        let bytes: Vec<_> = [0xff, 0xfe]
            .into_iter()
            .chain(xml.encode_utf16().flat_map(u16::to_le_bytes))
            .collect();
        let entries = parse(&bytes, chrono_tz::Asia::Shanghai).unwrap();
        assert_eq!(entries[0].text, "中文正文");
        assert!(
            parse(
                b"<?xml version='1.0' encoding='GBK'?><rss>\x81</rss>",
                chrono_tz::Asia::Shanghai
            )
            .is_err()
        );
    }
    #[test]
    fn atom_xhtml_body_preserves_nested_text_and_images() {
        for prefix in ["", "x:"] {
            let declaration = if prefix.is_empty() {
                "xmlns=\"http://www.w3.org/1999/xhtml\""
            } else {
                "xmlns:x=\"http://www.w3.org/1999/xhtml\""
            };
            let xml = format!(
                r#"<feed xmlns="http://www.w3.org/2005/Atom"><title>Feed</title><entry><id>unique</id><title>Fallback</title><link href="https://weibo.com/status/123"/><content type="xhtml">
                <{prefix}div {declaration}><{prefix}p>Nested <{prefix}b>正文</{prefix}b> &amp; &lt;literal&gt;</{prefix}p><{prefix}img src="https://example.org/a.png?x=1&amp;y=2"/></{prefix}div>
            </content><summary>Wrong summary</summary></entry></feed>"#
            );
            let entries = parse(xml.as_bytes(), chrono_tz::Asia::Shanghai).unwrap();
            assert_eq!(entries[0].text, "Nested 正文 & <literal>");
            assert_eq!(entries[0].images, vec!["https://example.org/a.png?x=1&y=2"]);
        }
    }
    #[test]
    fn rss_atom_keys_and_content() {
        let items = parse(br#"<rss xmlns:content="http://purl.org/rss/1.0/modules/content/"><channel><item><link>https://weibo.com/status/123</link><title>Fallback</title><description>Summary</description><content:encoded>&lt;p&gt;Body &amp;amp; text&lt;/p&gt;&lt;img src="https://example.org/a.png"/&gt;</content:encoded><pubDate>Thu, 26 Nov 2026 12:00:00 +0000</pubDate></item></channel></rss>"#, chrono_tz::Asia::Shanghai).unwrap();
        assert_eq!(items[0].key, "weibo:123");
        assert_eq!(items[0].text, "Body & text");
        assert_eq!(items[0].images, vec!["https://example.org/a.png"]);
        assert_eq!(items[0].timestamp, 1795694400);
        let items = parse(br#"<feed xmlns="http://www.w3.org/2005/Atom"><entry><link rel="self" href="https://example.org/self"/><link rel="alternate" href="https://example.org/post"/><title>Title</title><author><name>Author</name></author></entry></feed>"#, chrono_tz::Asia::Shanghai).unwrap();
        assert_eq!(
            items[0].key,
            format!(
                "weibo:{}",
                &hex::encode(Sha1::digest(b"https://example.org/post"))[..12]
            )
        );
        assert_eq!(items[0].author, "Author");
        assert_eq!(items[0].text, "Title");
        assert!(parse(br#"<!DOCTYPE rss [<!ENTITY x SYSTEM "file:///etc/passwd">]><rss><channel><item><title>&x;</title></item></channel></rss>"#, chrono_tz::Asia::Shanghai).is_err());
        assert!(parse(b"<html/>", chrono_tz::Asia::Shanghai).is_err());
        assert!(
            parse(b"<feed/>", chrono_tz::Asia::Shanghai)
                .unwrap()
                .is_empty()
        );
    }
}
