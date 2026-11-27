mod export;
use super::Plugin;
use crate::{
    api::App,
    config::Settings,
    error::{Error, Result},
    media, network, tasks,
};
use anyhow::bail;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use roxmltree::Node;
use serde::Deserialize;
use serde_json::{Value, json};
use sha1::{Digest, Sha1};
use sqlx::{FromRow, Row};
use std::{collections::HashMap, path::PathBuf, sync::LazyLock};

pub struct Rss;
static MANIFEST: LazyLock<Value> =
    LazyLock::new(|| serde_json::from_str(include_str!("manifest.json")).unwrap());
#[derive(Clone, FromRow)]
struct Feed {
    id: String,
    url: String,
    title: String,
    alias: String,
    icon: String,
}
struct Entry {
    key: String,
    title: String,
    link: String,
    text: String,
    timestamp: i64,
    images: Vec<String>,
}
#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct Sync {
    all: bool,
    feed: Option<String>,
}
fn short(value: &str, length: usize) -> String {
    value.chars().take(length).collect()
}
fn hash(value: &str, length: usize) -> String {
    hex::encode(Sha1::digest(value.as_bytes()))[..length].into()
}
fn plain(value: &str) -> String {
    scraper::Html::parse_fragment(value)
        .root_element()
        .text()
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
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
fn parse(xml: &[u8], timezone: chrono_tz::Tz) -> anyhow::Result<(String, Vec<Entry>)> {
    let xml = super::feed::decode(xml)?;
    let doc =
        roxmltree::Document::parse(&xml).map_err(|_| anyhow::anyhow!("不是有效的 RSS/Atom XML"))?;
    let root = doc.root_element();
    let container = child(root, "channel").unwrap_or(root);
    let title = super::feed::title(container);
    let nodes = if child(container, "item").is_some() {
        container
    } else {
        root
    };
    let mut entries = Vec::new();
    for node in nodes
        .children()
        .filter(|n| n.is_element() && matches!(n.tag_name().name(), "item" | "entry"))
    {
        let title = super::feed::title(node);
        let link = node
            .children()
            .filter(|n| n.is_element() && n.tag_name().name() == "link")
            .find_map(|n| {
                if n.attribute("rel").is_none_or(|s| s == "alternate") {
                    n.attribute("href").or(n.text())
                } else {
                    None
                }
            })
            .unwrap_or("")
            .trim()
            .to_owned();
        let guid = ["guid", "id"]
            .iter()
            .map(|name| text(node, name))
            .find(|s| !s.is_empty())
            .unwrap_or_default();
        let body_node = ["encoded", "content", "description", "summary"]
            .iter()
            .filter_map(|name| child(node, name))
            .find(|n| {
                n.descendants()
                    .any(|part| part.is_text() && !part.text().unwrap_or("").trim().is_empty())
                    || n.attribute("type") == Some("xhtml")
            });
        let xhtml = body_node.filter(|n| n.attribute("type") == Some("xhtml"));
        let html = body_node.and_then(|n| n.text()).unwrap_or("").trim();
        let date = ["pubDate", "published", "updated", "date"]
            .iter()
            .map(|name| text(node, name))
            .find(|s| !s.is_empty())
            .unwrap_or_default();
        let timestamp = super::feed::timestamp(&date, timezone);
        if link.is_empty() && guid.is_empty() && title.is_empty() {
            continue;
        }
        let seed = if !guid.is_empty() {
            guid
        } else if !link.is_empty() {
            link.clone()
        } else {
            format!("{title}|{}", timestamp.unwrap_or(0))
        };
        let mut images = Vec::new();
        if let Some(enclosure) = node.children().find(|n| {
            n.is_element()
                && n.tag_name().name() == "enclosure"
                && n.attribute("type").is_some_and(|s| s.starts_with("image/"))
        }) && let Some(url) = enclosure
            .attribute("url")
            .filter(|s| network::parse_url(s).is_ok())
        {
            images.push(url.into());
        }
        let html_document = scraper::Html::parse_fragment(html);
        let selector = scraper::Selector::parse("img[src]").unwrap();
        let body_images: Vec<_> = if let Some(body) = xhtml {
            body.descendants()
                .filter(|n| n.is_element() && n.tag_name().name() == "img")
                .filter_map(|n| n.attribute("src"))
                .collect()
        } else {
            html_document
                .select(&selector)
                .filter_map(|n| n.value().attr("src"))
                .collect()
        };
        for src in body_images {
            if images.len() >= 3 {
                break;
            }
            if network::parse_url(src).is_ok() && !images.iter().any(|s| s == src) {
                images.push(src.into());
            }
        }
        entries.push(Entry {
            key: hash(&seed, 16),
            title,
            link,
            text: if let Some(body) = xhtml {
                body.descendants()
                    .filter(|n| n.is_text())
                    .filter_map(|n| n.text())
                    .collect::<String>()
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" ")
            } else {
                plain(html)
            },
            timestamp: timestamp.unwrap_or_else(|| Utc::now().timestamp()),
            images,
        });
    }
    if entries.is_empty() {
        bail!("订阅源里没有可识别的条目");
    }
    Ok((title, entries))
}

#[async_trait]
impl Plugin for Rss {
    fn render_post(&self, post: &Value) -> Option<String> {
        Some(export::post(post))
    }
    fn export_page(&self, data: &Value) -> Option<String> {
        Some(export::page(data))
    }
    fn id(&self) -> &'static str {
        "rss"
    }
    fn manifest(&self) -> &'static Value {
        &MANIFEST
    }
    async fn migrate(&self, app: &App) -> anyhow::Result<()> {
        sqlx::query("CREATE TABLE IF NOT EXISTS rss_feeds (id TEXT PRIMARY KEY,url TEXT NOT NULL,title TEXT NOT NULL DEFAULT '',alias TEXT NOT NULL DEFAULT '',created_at TEXT NOT NULL,last_sync INTEGER NOT NULL DEFAULT 0,last_error TEXT NOT NULL DEFAULT '',icon TEXT NOT NULL DEFAULT '')").execute(&app.db).await?;
        if !sqlx::query("PRAGMA table_info(rss_feeds)")
            .fetch_all(&app.db)
            .await?
            .iter()
            .any(|r| r.get::<String, _>("name") == "icon")
        {
            sqlx::query("ALTER TABLE rss_feeds ADD COLUMN icon TEXT NOT NULL DEFAULT ''")
                .execute(&app.db)
                .await?;
        }
        sqlx::query("CREATE TABLE IF NOT EXISTS rss_pending_media (reference TEXT PRIMARY KEY)")
            .execute(&app.db)
            .await?;
        let pending: Vec<String> = sqlx::query_scalar("SELECT reference FROM rss_pending_media")
            .fetch_all(&app.db)
            .await?;
        for reference in pending {
            media::delete_unused(app, &reference).await?;
        }
        sqlx::query("DELETE FROM rss_pending_media")
            .execute(&app.db)
            .await?;
        Ok(())
    }
    async fn data(
        &self,
        app: &App,
        params: &HashMap<String, String>,
        admin: bool,
    ) -> Result<Value> {
        let rows=sqlx::query("SELECT f.*, (SELECT COUNT(*) FROM posts p WHERE p.source='rss' AND p.source_key LIKE 'rss:'||f.id||':%') AS count FROM rss_feeds f ORDER BY created_at,id").fetch_all(&app.db).await?;
        let feeds:Vec<Value>=rows.into_iter().map(|row| {
            let alias:String=row.get("alias"); let title:String=row.get("title"); let url:String=row.get("url");
            let mut value=json!({"id":row.get::<String,_>("id"),"title":if !alias.is_empty(){alias}else if !title.is_empty(){title}else{url.clone()},"count":row.get::<i64,_>("count")});
            if admin {value["url"]=json!(url);value["last_sync"]=json!(row.get::<i64,_>("last_sync"));value["last_error"]=json!(row.get::<String,_>("last_error"));}
            value
        }).collect();
        let feed = params.get("feed").filter(|s| valid_id(s));
        let mut result = crate::api::posts_filtered(
            app,
            params,
            Some(("rss", feed.map(|s| format!("rss:{s}:%")))),
        )
        .await?;
        result["feeds"] = json!(feeds);
        Ok(result)
    }
    async fn export_snapshot(&self, app: &App) -> Result<Option<super::ExportSnapshot>> {
        let feeds = sqlx::query("SELECT f.id,CASE WHEN f.alias<>'' THEN f.alias WHEN f.title<>'' THEN f.title ELSE f.url END AS title,(SELECT COUNT(*) FROM posts p WHERE p.source='rss' AND p.source_key LIKE 'rss:'||f.id||':%') AS count FROM rss_feeds f ORDER BY created_at,id")
            .fetch_all(&app.db).await?.into_iter().map(crate::api::row_json).collect::<Vec<_>>();
        Ok(Some(super::ExportSnapshot {
            data: json!({"feeds":feeds}),
            references: Vec::new(),
        }))
    }
    fn export_data(
        &self,
        data: &Value,
        posts: &[Value],
        _media: &std::collections::BTreeMap<String, Value>,
    ) -> Value {
        let posts: Vec<_> = posts
            .iter()
            .filter(|post| post["source"] == self.id())
            .collect();
        json!({"items":{"feeds":data["feeds"],"posts":posts},"meta":{"total":posts.len()}})
    }
    async fn admin(&self, app: &App, input: Value) -> Result<Value> {
        match input["op"].as_str().unwrap_or("") {
            "add" => {
                let _operation = super::mutation_guard(self.id())?;
                let url = input["url"].as_str().unwrap_or("").trim();
                if url.chars().count() > 500 {
                    return Err(Error::invalid("订阅地址过长"));
                }
                network::parse_url(url).map_err(|e| Error::invalid(e.to_string()))?;
                let xml = app
                    .network
                    .get(
                        url,
                        true,
                        8 * 1024 * 1024,
                        &tokio_util::sync::CancellationToken::new(),
                    )
                    .await
                    .map_err(|e| Error::invalid(e.to_string()))?;
                let (title, _) =
                    parse(&xml, app.config.timezone).map_err(|e| Error::invalid(e.to_string()))?;
                let alias = short(input["alias"].as_str().unwrap_or("").trim(), 40);
                let id = hash(url, 12);
                {
                    let _guard = app.content_lock.write().await;
                    let count=sqlx::query("INSERT INTO rss_feeds(id,url,title,alias,created_at) VALUES(?,?,?,?,?) ON CONFLICT(id) DO NOTHING").bind(&id).bind(url).bind(&title).bind(&alias).bind(app.config.now()).execute(&app.db).await?.rows_affected();
                    if count == 0 {
                        return Err(Error::invalid("该订阅源已在列表中"));
                    }
                }
                let mut result = super::enqueue(app, "rss", json!({"feed":id,"all":false})).await?;
                result["id"] = json!(id);
                result["title"] = json!(if alias.is_empty() { title } else { alias });
                Ok(result)
            }
            "sync" => {
                let id = feed_id(&input)?;
                if !sqlx::query_scalar::<_, bool>(
                    "SELECT EXISTS(SELECT 1 FROM rss_feeds WHERE id=?)",
                )
                .bind(id)
                .fetch_one(&app.db)
                .await?
                {
                    return Err(Error::not_found("订阅源不存在"));
                }
                super::enqueue(app, "rss", json!({"feed":id,"all":false})).await
            }
            "remove" => {
                let _operation = super::mutation_guard(self.id())?;
                remove(
                    app,
                    feed_id(&input)?,
                    input["purge"] == 1 || input["purge"] == true,
                )
                .await
            }
            _ => Err(Error::invalid("操作无效")),
        }
    }
    async fn sync(&self, app: &App, ctx: &tasks::Context, params: Value) -> anyhow::Result<Value> {
        let options: Sync = serde_json::from_value(params)?;
        let config = super::config(self, &Settings::load(app).await?);
        cleanup(app, &config).await?;
        let feeds=sqlx::query_as::<_,Feed>("SELECT id,url,title,alias,icon FROM rss_feeds WHERE (? IS NULL OR id=?) ORDER BY created_at,id").bind(&options.feed).bind(&options.feed).fetch_all(&app.db).await?;
        let total = feeds.len();
        let mut waiting = feeds.into_iter();
        let mut running = tokio::task::JoinSet::new();
        let mut added = 0;
        let mut failed = 0;
        let mut completed = 0;
        loop {
            while running.len() < 4 && !ctx.stop.is_cancelled() {
                let Some(feed) = waiting.next() else {
                    break;
                };
                let app = app.clone();
                let ctx = ctx.clone();
                let all = options.all;
                running.spawn(async move {
                    let config = super::config(&Rss, &Settings::load(&app).await?);
                    let result = collect(&app, &ctx, &feed, &config, all).await;
                    if let Err(error) = &result {
                        sqlx::query("UPDATE rss_feeds SET last_error=? WHERE id=?")
                            .bind(short(&error.to_string(), 200))
                            .bind(&feed.id)
                            .execute(&app.db)
                            .await?;
                    }
                    Ok::<_, anyhow::Error>(result)
                });
            }
            let Some(result) = running.join_next().await else {
                break;
            };
            match result {
                Ok(Ok(Ok(n))) => added += n,
                Ok(Ok(Err(_))) => failed += 1,
                fatal => {
                    ctx.stop.cancel();
                    while running.join_next().await.is_some() {}
                    bail!("订阅任务执行中断：{fatal:?}");
                }
            }
            completed += 1;
            if !ctx.stop.is_cancelled()
                && let Err(error) = ctx
                    .progress("同步订阅源", completed, Some(total as i64))
                    .await
            {
                ctx.stop.cancel();
                while running.join_next().await.is_some() {}
                return Err(error);
            }
        }
        ctx.check()?;
        if failed > 0 && failed == total {
            bail!("全部订阅源同步失败，请查看各源错误");
        }
        Ok(json!({"added":added,"failed_feeds":failed}))
    }
    async fn references(&self, app: &App, reference: &str) -> Result<bool> {
        Ok(sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM rss_feeds WHERE icon=? UNION ALL SELECT 1 FROM posts WHERE json_valid(meta) AND json_extract(meta,'$.avatar')=?)").bind(reference).bind(reference).fetch_one(&app.db).await?)
    }
    fn post_refs(&self, meta: &Value) -> Vec<String> {
        meta["avatar"]
            .as_str()
            .filter(|s| !s.is_empty())
            .map(|s| vec![s.into()])
            .unwrap_or_default()
    }
    async fn posts_purged(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    ) -> Result<Vec<String>> {
        let refs = sqlx::query_scalar("SELECT icon FROM rss_feeds WHERE icon<>''")
            .fetch_all(&mut **tx)
            .await?;
        sqlx::query("UPDATE rss_feeds SET icon=''")
            .execute(&mut **tx)
            .await?;
        Ok(refs)
    }
    async fn delete_media(&self, app: &App, reference: &str) -> Result<()> {
        if let Some(name) = reference
            .strip_prefix("up:")
            .filter(|s| crate::api::upload_name(s))
        {
            media::remove_file(&app.data.join("rss-thumbs").join(format!("{name}.webp"))).await?;
        }
        Ok(())
    }
    async fn thumbnail(&self, app: &App, reference: &str) -> Option<PathBuf> {
        let name = reference
            .strip_prefix("up:")
            .filter(|s| crate::api::upload_name(s))?;
        let root = tokio::fs::canonicalize(app.data.join("rss-thumbs"))
            .await
            .ok()?;
        let path = tokio::fs::canonicalize(root.join(format!("{name}.webp")))
            .await
            .ok()?;
        path.starts_with(root).then_some(path)
    }
}
fn valid_id(id: &str) -> bool {
    id.len() == 12
        && id
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn feed_id(input: &Value) -> Result<&str> {
    input["feed_id"]
        .as_str()
        .filter(|s| valid_id(s))
        .ok_or_else(|| Error::invalid("订阅源标识无效"))
}

async fn cleanup(app: &App, config: &Settings) -> Result<()> {
    if !config.enabled("auto_delete") {
        return Ok(());
    }
    let cutoff = Utc::now().timestamp() - config.number("retention_days").max(1) * 86400;
    let _guard = app.content_lock.write().await;
    let mut tx = app.db.begin_with("BEGIN IMMEDIATE").await?;
    let (_, refs) = delete_posts(
        &mut tx,
        "unixepoch(created_at) < CAST(? AS INTEGER)",
        &cutoff.to_string(),
        false,
    )
    .await?;
    tx.commit().await?;
    for reference in refs {
        media::delete_unused(app, &reference).await?;
    }
    Ok(())
}
async fn delete_posts(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    condition: &str,
    value: &str,
    tombstones: bool,
) -> Result<(usize, Vec<String>)> {
    let rows = sqlx::query(&format!(
        "SELECT id,images,meta FROM posts WHERE source='rss' AND {condition}"
    ))
    .bind(value)
    .fetch_all(&mut **tx)
    .await?;
    let mut references = Vec::new();
    for row in &rows {
        let id: i64 = row.get("id");
        references.extend(
            serde_json::from_str::<Vec<String>>(&row.get::<String, _>("images"))
                .unwrap_or_default(),
        );
        if let Ok(meta) = serde_json::from_str::<Value>(&row.get::<String, _>("meta"))
            && let Some(avatar) = meta["avatar"].as_str()
        {
            references.push(avatar.into());
        }
        sqlx::query("DELETE FROM comments WHERE kind='post' AND target_id=?")
            .bind(id)
            .execute(&mut **tx)
            .await?;
        sqlx::query("DELETE FROM post_likes WHERE post_id=?")
            .bind(id)
            .execute(&mut **tx)
            .await?;
        sqlx::query("DELETE FROM posts WHERE id=?")
            .bind(id)
            .execute(&mut **tx)
            .await?;
    }
    if tombstones {
        sqlx::query("DELETE FROM plugin_tombstones WHERE plugin='rss' AND key LIKE ?")
            .bind(value)
            .execute(&mut **tx)
            .await?;
    }
    Ok((rows.len(), references))
}
async fn remove(app: &App, id: &str, purge: bool) -> Result<Value> {
    let _guard = app.content_lock.write().await;
    let mut tx = app.db.begin_with("BEGIN IMMEDIATE").await?;
    let feed =
        sqlx::query_as::<_, Feed>("SELECT id,url,title,alias,icon FROM rss_feeds WHERE id=?")
            .bind(id)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or_else(|| Error::not_found("订阅源不存在"))?;
    sqlx::query("DELETE FROM rss_feeds WHERE id=?")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    let (deleted, mut refs) = if purge {
        delete_posts(&mut tx, "source_key LIKE ?", &format!("rss:{id}:%"), true).await?
    } else {
        (0, Vec::new())
    };
    tx.commit().await?;
    if !feed.icon.is_empty() {
        refs.push(feed.icon);
    }
    for reference in refs {
        media::delete_unused(app, &reference).await?;
    }
    Ok(
        json!({"ok":true,"removed":if feed.alias.is_empty(){feed.title}else{feed.alias},"deleted":deleted}),
    )
}

async fn fetch_identity(
    app: &App,
    ctx: &tasks::Context,
    feed: &Feed,
    name: &str,
    host: &str,
) -> anyhow::Result<String> {
    let mut url = url::Url::parse("https://www.google.com/s2/favicons")?;
    url.query_pairs_mut()
        .append_pair("sz", "64")
        .append_pair("domain", network::parse_url(&feed.url)?.host_str().unwrap());
    let image = match fetch_image(app, ctx, url.as_str(), 0).await {
        Ok(image) => image,
        Err(_) => {
            ctx.check()?;
            return Ok(String::new());
        }
    };
    let reference = format!("up:{image}");
    let _guard = app.content_lock.write().await;
    ctx.check()?;
    sqlx::query("INSERT INTO rss_pending_media(reference) VALUES(?)")
        .bind(&reference)
        .execute(&app.db)
        .await?;
    let result = async {
        tokio::fs::create_dir_all(app.data.join("rss-thumbs")).await?;
        tokio::fs::copy(
            ctx.directory.join(&image),
            app.data.join("uploads").join(&image),
        )
        .await?;
        tokio::fs::copy(
            ctx.directory.join(format!("{image}.thumb.webp")),
            app.data.join("rss-thumbs").join(format!("{image}.webp")),
        )
        .await?;
        let mut tx = app.db.begin_with("BEGIN IMMEDIATE").await?;
        ctx.check()?;
        let updated = sqlx::query("UPDATE rss_feeds SET icon=? WHERE id=?")
            .bind(&reference)
            .bind(&feed.id)
            .execute(&mut *tx)
            .await?
            .rows_affected();
        if updated == 0 {
            bail!("订阅源已移除");
        }
        let rows =
            sqlx::query("SELECT id,meta FROM posts WHERE source='rss' AND source_key LIKE ?")
                .bind(format!("rss:{}:%", feed.id))
                .fetch_all(&mut *tx)
                .await?;
        for row in rows {
            let mut meta = serde_json::from_str::<Value>(&row.get::<String, _>("meta"))
                .ok()
                .filter(Value::is_object)
                .unwrap_or_else(|| json!({}));
            meta["avatar"] = json!(reference);
            meta["name"] = json!(short(name, 40));
            meta["handle"] = json!(host);
            sqlx::query("UPDATE posts SET meta=? WHERE id=?")
                .bind(meta.to_string())
                .bind(row.get::<i64, _>("id"))
                .execute(&mut *tx)
                .await?;
        }
        sqlx::query("DELETE FROM rss_pending_media WHERE reference=?")
            .bind(&reference)
            .execute(&mut *tx)
            .await?;
        ctx.check()?;
        tx.commit().await?;
        Ok::<_, anyhow::Error>(())
    }
    .await;
    if result.is_err() {
        media::delete_unused(app, &reference).await?;
        sqlx::query("DELETE FROM rss_pending_media WHERE reference=?")
            .bind(&reference)
            .execute(&app.db)
            .await?;
    }
    result?;
    Ok(reference)
}

async fn collect(
    app: &App,
    ctx: &tasks::Context,
    feed: &Feed,
    config: &Settings,
    all: bool,
) -> anyhow::Result<usize> {
    ctx.check()?;
    let xml = app
        .network
        .get(&feed.url, true, 8 * 1024 * 1024, &ctx.stop)
        .await?;
    let (title, entries) = parse(&xml, app.config.timezone)?;
    let host = network::parse_url(&feed.url)?
        .host_str()
        .unwrap()
        .trim_start_matches("www.")
        .to_owned();
    let name = if !feed.alias.is_empty() {
        &feed.alias
    } else if !title.is_empty() {
        &title
    } else {
        &host
    };
    let icon = if feed.icon.is_empty() {
        fetch_identity(app, ctx, feed, name, &host).await?
    } else {
        feed.icon.clone()
    };
    let cutoff = Utc::now().timestamp() - config.number("retention_days").max(1) * 86400;
    let mut added = 0;
    let limit = config
        .number(if all { "history_limit" } else { "max_items" })
        .max(1) as usize;
    for entry in entries.into_iter().take(limit) {
        ctx.check()?;
        if config.enabled("auto_delete") && entry.timestamp < cutoff {
            continue;
        }
        let key = format!("rss:{}:{}", feed.id, entry.key);
        let known:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM posts WHERE source='rss' AND source_key=? UNION ALL SELECT 1 FROM plugin_tombstones WHERE plugin='rss' AND key=?)").bind(&key).bind(&key).fetch_one(&app.db).await?;
        if known {
            continue;
        }
        let content = if entry.title.is_empty() {
            short(&entry.text, 100)
        } else {
            short(&entry.title, 500)
        };
        if content.is_empty() {
            continue;
        }
        let mut images = Vec::new();
        for url in &entry.images {
            ctx.check()?;
            if let Ok(name) = fetch_image(app, ctx, url, 120).await {
                images.push(name);
            }
        }
        let meta = json!({"card":"rss","feed":feed.id,"name":short(name,40),"title":short(&entry.title,120),"summary":short(&entry.text,200),"avatar":icon,"handle":host});
        let created = DateTime::from_timestamp(entry.timestamp, 0)
            .unwrap_or_else(Utc::now)
            .with_timezone(&app.config.timezone)
            .to_rfc3339();
        let _guard = app.content_lock.write().await;
        ctx.check()?;
        let still_subscribed: bool =
            sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM rss_feeds WHERE id=?)")
                .bind(&feed.id)
                .fetch_one(&app.db)
                .await?;
        if !still_subscribed {
            break;
        }
        let refs: Vec<String> = images.iter().map(|name| format!("up:{name}")).collect();
        for reference in &refs {
            sqlx::query("INSERT INTO rss_pending_media(reference) VALUES(?)")
                .bind(reference)
                .execute(&app.db)
                .await?;
        }
        let result=async {
            tokio::fs::create_dir_all(app.data.join("rss-thumbs")).await?;
            for name in &images {
                ctx.check()?;
                tokio::fs::copy(ctx.directory.join(name),app.data.join("uploads").join(name)).await?;
                tokio::fs::copy(ctx.directory.join(format!("{name}.thumb.webp")),app.data.join("rss-thumbs").join(format!("{name}.webp"))).await?;
            }
            let mut tx=app.db.begin_with("BEGIN IMMEDIATE").await?;
            ctx.check()?;
            let inserted=sqlx::query("INSERT INTO posts(content,created_at,updated_at,images,meta,source,source_key,link) SELECT ?,?,?,?,?,'rss',?,? WHERE NOT EXISTS(SELECT 1 FROM posts WHERE source='rss' AND source_key=?) AND NOT EXISTS(SELECT 1 FROM plugin_tombstones WHERE plugin='rss' AND key=?)")
                .bind(&content).bind(&created).bind(&created).bind(json!(refs).to_string()).bind(meta.to_string()).bind(&key).bind(if network::parse_url(&entry.link).is_ok(){&entry.link}else{""}).bind(&key).bind(&key).execute(&mut *tx).await?.rows_affected();
            for reference in &refs {sqlx::query("DELETE FROM rss_pending_media WHERE reference=?").bind(reference).execute(&mut *tx).await?;}
            tx.commit().await?;
            Ok::<_,anyhow::Error>(inserted)
        }.await;
        if !matches!(&result, Ok(n) if *n > 0) {
            for reference in &refs {
                media::delete_unused(app, reference).await?;
                sqlx::query("DELETE FROM rss_pending_media WHERE reference=?")
                    .bind(reference)
                    .execute(&app.db)
                    .await?;
            }
        }
        added += result? as usize;
    }
    ctx.check()?;
    let _guard = app.content_lock.write().await;
    sqlx::query("UPDATE rss_feeds SET title=?,last_sync=?,last_error='' WHERE id=?")
        .bind(title)
        .bind(Utc::now().timestamp())
        .bind(&feed.id)
        .execute(&app.db)
        .await?;
    Ok(added)
}
async fn fetch_image(
    app: &App,
    ctx: &tasks::Context,
    url: &str,
    min_size: u32,
) -> anyhow::Result<String> {
    let bytes = app
        .network
        .get(url, false, 16 * 1024 * 1024, &ctx.stop)
        .await?;
    let name = format!("{}.webp", &crate::auth::random_token()?[..32]);
    let source = ctx.directory.join(format!("{name}.source"));
    tokio::fs::write(&source, bytes).await?;
    let target = ctx.directory.join(&name);
    let thumb = ctx.directory.join(format!("{name}.thumb.webp"));
    let result=async {
        let (width,height,_)=media::inspect(app,&source,&ctx.stop).await?;
        if width<min_size || height<min_size {bail!("忽略小图片");}
        let _permit=tokio::select!{_=ctx.stop.cancelled()=>bail!("任务已取消"),permit=app.media_slots.acquire()=>permit?};
        media::compress(source.to_str().unwrap(),&target,100*1024,1600,&ctx.stop).await?;
        media::compress(source.to_str().unwrap(),&thumb,50*1024,480,&ctx.stop).await?;
        Ok::<_,anyhow::Error>(name)
    }.await;
    media::remove_file(&source).await?;
    if result.is_err() {
        media::remove_file(&target).await?;
        media::remove_file(&thumb).await?;
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn favicon_backfill_rollback_cache_and_recovery() {
        use crate::{config::Config, database};
        use std::sync::{Arc, Mutex};
        use tokio_util::sync::CancellationToken;
        let root = tempfile::tempdir().unwrap();
        let data = root.path().join("data");
        let work = root.path().join("work");
        let fixture = Arc::new(Mutex::new(network::TestResponses::default()));
        let mut client = network::Client::new(None);
        client.fixture = Some(fixture.clone());
        let app = App {
            db: database::open(&data).await.unwrap(),
            tasks: tasks::open(&work).await.unwrap(),
            network: client,
            data,
            config: Arc::new(Config::default()),
            shutdown: CancellationToken::new(),
            password_slots: Arc::new(tokio::sync::Semaphore::new(2)),
            media_slots: Arc::new(tokio::sync::Semaphore::new(2)),
            content_lock: Arc::new(tokio::sync::RwLock::new(())),
        };
        crate::plugins::migrate(&app).await.unwrap();
        let ctx = tasks::Context {
            id: 1,
            pool: app.tasks.clone(),
            directory: work.join("job-1"),
            stop: CancellationToken::new(),
        };
        std::fs::create_dir_all(&ctx.directory).unwrap();
        let mut feed = Feed {
            id: "0123456789ab".into(),
            url: "https://www.example.org/feed".into(),
            title: "".into(),
            alias: "Alias".into(),
            icon: "".into(),
        };
        sqlx::query("INSERT INTO rss_feeds(id,url,created_at) VALUES(?,?,'2026-11-26')")
            .bind(&feed.id)
            .bind(&feed.url)
            .execute(&app.db)
            .await
            .unwrap();
        let icon_url = "https://www.google.com/s2/favicons?sz=64&domain=www.example.org";
        {
            let mut fixture = fixture.lock().unwrap();
            fixture.responses.insert(feed.url.clone(), b"<rss><channel><title>Feed</title><item><guid>one</guid><title>Post</title></item></channel></rss>".to_vec());
            fixture
                .responses
                .insert(icon_url.into(), b"not an image".to_vec());
        }
        let config = super::super::config(&Rss, &Settings::load(&app).await.unwrap());
        assert_eq!(collect(&app, &ctx, &feed, &config, false).await.unwrap(), 1);
        let icon: String = sqlx::query_scalar("SELECT icon FROM rss_feeds")
            .fetch_one(&app.db)
            .await
            .unwrap();
        assert!(icon.is_empty());
        let picture = root.path().join("icon.png");
        image::RgbImage::new(64, 64).save(&picture).unwrap();
        fixture
            .lock()
            .unwrap()
            .responses
            .insert(icon_url.into(), std::fs::read(picture).unwrap());
        sqlx::query("CREATE TRIGGER fail_identity BEFORE UPDATE ON posts BEGIN SELECT RAISE(ABORT,'identity failure'); END")
            .execute(&app.db).await.unwrap();
        assert!(collect(&app, &ctx, &feed, &config, false).await.is_err());
        let icon: String = sqlx::query_scalar("SELECT icon FROM rss_feeds")
            .fetch_one(&app.db)
            .await
            .unwrap();
        assert!(icon.is_empty());
        assert_eq!(
            std::fs::read_dir(app.data.join("uploads")).unwrap().count(),
            0
        );
        assert_eq!(
            std::fs::read_dir(app.data.join("rss-thumbs"))
                .unwrap()
                .count(),
            0
        );
        let pending: i64 = sqlx::query_scalar("SELECT count(*) FROM rss_pending_media")
            .fetch_one(&app.db)
            .await
            .unwrap();
        assert_eq!(pending, 0);
        sqlx::query("DROP TRIGGER fail_identity")
            .execute(&app.db)
            .await
            .unwrap();
        assert_eq!(collect(&app, &ctx, &feed, &config, false).await.unwrap(), 0);
        feed.icon = sqlx::query_scalar("SELECT icon FROM rss_feeds")
            .fetch_one(&app.db)
            .await
            .unwrap();
        assert!(feed.icon.starts_with("up:"));
        let meta: String = sqlx::query_scalar("SELECT meta FROM posts")
            .fetch_one(&app.db)
            .await
            .unwrap();
        let meta: Value = serde_json::from_str(&meta).unwrap();
        assert_eq!(meta["avatar"], feed.icon);
        assert_eq!(meta["name"], "Alias");
        assert_eq!(meta["handle"], "example.org");
        assert_eq!(meta["title"], "Post");
        let original = app.data.join("uploads").join(&feed.icon[3..]);
        let thumbnail = app
            .data
            .join("rss-thumbs")
            .join(format!("{}.webp", &feed.icon[3..]));
        assert!(std::fs::metadata(&original).unwrap().len() < 100 * 1024);
        assert!(std::fs::metadata(&thumbnail).unwrap().len() < 50 * 1024);
        fixture.lock().unwrap().responses.insert(feed.url.clone(), b"<rss><channel><title>Feed</title><item><guid>two</guid><title>New post</title></item></channel></rss>".to_vec());
        let requests = fixture.lock().unwrap().requests.len();
        assert_eq!(collect(&app, &ctx, &feed, &config, false).await.unwrap(), 1);
        assert_eq!(fixture.lock().unwrap().requests.len(), requests + 1);
        let avatars: Vec<String> =
            sqlx::query_scalar("SELECT json_extract(meta,'$.avatar') FROM posts")
                .fetch_all(&app.db)
                .await
                .unwrap();
        assert_eq!(avatars, vec![feed.icon.clone(), feed.icon.clone()]);
        let orphan = format!("{}.webp", "f".repeat(32));
        std::fs::copy(&original, app.data.join("uploads").join(&orphan)).unwrap();
        std::fs::copy(
            &thumbnail,
            app.data.join("rss-thumbs").join(format!("{orphan}.webp")),
        )
        .unwrap();
        for reference in [format!("up:{orphan}"), feed.icon.clone()] {
            sqlx::query("INSERT INTO rss_pending_media(reference) VALUES(?)")
                .bind(reference)
                .execute(&app.db)
                .await
                .unwrap();
        }
        Rss.migrate(&app).await.unwrap();
        assert!(original.exists() && thumbnail.exists());
        assert!(!app.data.join("uploads").join(&orphan).exists());
        assert!(
            !app.data
                .join("rss-thumbs")
                .join(format!("{orphan}.webp"))
                .exists()
        );
        ctx.stop.cancel();
        assert!(collect(&app, &ctx, &feed, &config, false).await.is_err());
        assert_eq!(fixture.lock().unwrap().requests.len(), requests + 1);
    }
    #[test]
    fn atom_title_fallback_and_local_date() {
        let xml = br#"<feed xmlns="http://www.w3.org/2005/Atom"><title type="html">&lt;b&gt;Feed&lt;/b&gt;</title><entry><id>one</id><title type="xhtml"><div xmlns="http://www.w3.org/1999/xhtml">A <b>title</b> &amp; &lt;literal&gt;</div></title><link href="https://weibo.com/status/123"/><updated>2000-01-01 08:00:00</updated></entry></feed>"#;
        let (title, entries) = parse(xml, chrono_tz::Asia::Shanghai).unwrap();
        assert_eq!(title, "Feed");
        assert_eq!(entries[0].title, "A title & <literal>");
        assert_eq!(entries[0].timestamp, 946684800);
    }
    #[test]
    fn declared_gbk_and_utf16_feeds() {
        let xml = "<?xml version='1.0' encoding='GBK'?><rss><channel><title>订阅</title><item><guid>one</guid><title>标题</title><link>https://weibo.com/status/123</link><description>中文正文</description></item></channel></rss>";
        let (bytes, _, errors) = encoding_rs::GBK.encode(xml);
        assert!(!errors);
        let (_, entries) = parse(&bytes, chrono_tz::Asia::Shanghai).unwrap();
        assert_eq!(entries[0].text, "中文正文");
        let xml = xml.replace("GBK", "UTF-16");
        let bytes: Vec<_> = [0xff, 0xfe]
            .into_iter()
            .chain(xml.encode_utf16().flat_map(u16::to_le_bytes))
            .collect();
        let (_, entries) = parse(&bytes, chrono_tz::Asia::Shanghai).unwrap();
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
            let (_, entries) = parse(xml.as_bytes(), chrono_tz::Asia::Shanghai).unwrap();
            assert_eq!(entries[0].text, "Nested 正文 & <literal>");
            assert_eq!(entries[0].images, vec!["https://example.org/a.png?x=1&y=2"]);
        }
    }
    #[test]
    fn rss_atom_rdf_identity_and_entities() {
        let (title,entries)=parse(br#"<rss><channel><title>A &amp; B</title><item><guid>unique</guid><title>Hello</title><description>&lt;p&gt;text &amp;amp; words&lt;/p&gt;</description><pubDate>Thu, 26 Nov 2026 12:00:00 +0000</pubDate></item></channel></rss>"#, chrono_tz::Asia::Shanghai).unwrap();
        assert_eq!(title, "A & B");
        assert_eq!(entries[0].key, hash("unique", 16));
        assert_eq!(entries[0].text, "text & words");
        let (_,entries)=parse(br#"<feed xmlns="http://www.w3.org/2005/Atom"><title>Atom</title><entry><id>unique</id><title>Hello</title><link rel="self" href="https://example.org/self"/><link rel="alternate" href="https://example.org/post"/><updated>2026-11-26T12:00:00Z</updated></entry></feed>"#, chrono_tz::Asia::Shanghai).unwrap();
        assert_eq!(entries[0].key, hash("unique", 16));
        assert_eq!(entries[0].link, "https://example.org/post");
        assert!(parse(br#"<!DOCTYPE rss [<!ENTITY x SYSTEM "file:///etc/passwd">]><rss><channel><item><title>&x;</title></item></channel></rss>"#, chrono_tz::Asia::Shanghai).is_err());
        assert_eq!(parse(br#"<rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#"><channel><title>RDF</title></channel><item><title>News</title><link>https://example.org/</link></item></rdf:RDF>"#, chrono_tz::Asia::Shanghai).unwrap().1.len(),1);
    }
}
