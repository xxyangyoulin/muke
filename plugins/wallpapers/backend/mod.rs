mod operations;
mod uploads;
use super::Plugin;
use crate::{
    api::App,
    config::Settings,
    error::{Error, Result},
    media, tasks,
};
use anyhow::bail;
use async_trait::async_trait;
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::{Value, json};
use sqlx::FromRow;
use std::os::unix::fs::MetadataExt;
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::LazyLock,
};

pub struct Wallpapers;
static MANIFEST: LazyLock<Value> =
    LazyLock::new(|| serde_json::from_str(include_str!("manifest.json")).unwrap());
#[derive(FromRow, Serialize)]
struct Wall {
    id: String,
    path: String,
    name: String,
    width: i64,
    height: i64,
    bytes: i64,
    mtime: i64,
}
impl Wall {
    fn cache(&self, root: &Path, settings: &Settings) -> String {
        let version = crate::auth::digest(&format!(
            "{}|{}|{}|{}|{}",
            root.display(),
            self.mtime,
            self.bytes,
            settings.number("thumb_size"),
            settings.number("thumb_quality")
        ));
        format!("{}-{version}.webp", self.id)
    }
    fn url(&self) -> String {
        let query: String = url::form_urlencoded::Serializer::new(String::new())
            .append_pair("file", &self.path)
            .append_pair("v", &self.mtime.to_string())
            .finish();
        format!("/image?{query}")
    }
}
async fn root(app: &App) -> Result<PathBuf> {
    let settings = Settings::load(app).await?;
    let directory = settings.get("wallpaper_dir");
    if directory.is_empty() {
        return Err(Error::invalid("请先配置壁纸目录"));
    }
    let root = tokio::fs::canonicalize(directory)
        .await
        .map_err(|_| Error::invalid("壁纸目录不可读取，本次操作已取消"))?;
    if !tokio::fs::metadata(&root).await?.is_dir() {
        return Err(Error::invalid("壁纸目录不可读取"));
    }
    Ok(root)
}
async fn file(root: &Path, relative: &str) -> Result<PathBuf> {
    if relative.is_empty()
        || !Path::new(relative)
            .components()
            .all(|c| matches!(c, std::path::Component::Normal(_)))
    {
        return Err(Error::not_found("图片不存在"));
    }
    let path = tokio::fs::canonicalize(root.join(relative))
        .await
        .map_err(|_| Error::not_found("图片不存在"))?;
    if !path.starts_with(root) || !tokio::fs::metadata(&path).await?.is_file() {
        return Err(Error::not_found("图片不存在"));
    }
    Ok(path)
}
async fn forget(tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>, id: &str) -> Result<Vec<i64>> {
    let reference = format!("wall:{id}");
    let rows: Vec<(i64, String)> =
        sqlx::query_as("SELECT id,images FROM posts WHERE images LIKE ?")
            .bind(format!("%\"{reference}\"%"))
            .fetch_all(&mut **tx)
            .await?;
    let mut deleted = Vec::new();
    for (id, raw) in rows {
        let mut images: Vec<String> = serde_json::from_str(&raw).unwrap_or_default();
        images.retain(|image| image != &reference);
        if images.is_empty() {
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
            deleted.push(id);
        } else {
            sqlx::query("UPDATE posts SET images=? WHERE id=?")
                .bind(json!(images).to_string())
                .bind(id)
                .execute(&mut **tx)
                .await?;
        }
    }
    sqlx::query("DELETE FROM wallpapers WHERE id=?")
        .bind(id)
        .execute(&mut **tx)
        .await?;
    Ok(deleted)
}

#[async_trait]
impl Plugin for Wallpapers {
    fn export_page(&self, data: &Value) -> Option<String> {
        use crate::export::pages::text;
        let mut html = String::from(
            r#"<div class="gallery-hero"><h1>壁纸收藏。</h1><p>精选风景已随站点打包，可在线预览、下载原图。</p></div><div class="masonry-fallback">"#,
        );
        for wall in data.as_array().unwrap() {
            html.push_str(&format!(r#"<article class="media-tile"><a class="tile-preview" href="{}" aria-label="查看 {}"><img src="{}" alt="{}" loading="lazy"></a></article>"#, text(wall,"original"), text(wall,"name"), text(wall,"thumbnail"), text(wall,"name")));
        }
        html.push_str("</div>");
        Some(html)
    }
    fn id(&self) -> &'static str {
        "wallpapers"
    }
    fn manifest(&self) -> &'static Value {
        &MANIFEST
    }
    fn interval(&self, config: &Settings) -> i64 {
        if config.get("wallpaper_dir").is_empty() {
            0
        } else {
            config.number("wallpaper_sync_interval")
        }
    }
    async fn migrate(&self, app: &App) -> anyhow::Result<()> {
        sqlx::query("CREATE TABLE IF NOT EXISTS wallpapers(id TEXT PRIMARY KEY,path TEXT NOT NULL,name TEXT NOT NULL,width INTEGER NOT NULL,height INTEGER NOT NULL,bytes INTEGER NOT NULL,mtime INTEGER NOT NULL,post_id INTEGER)").execute(&app.db).await?;
        operations::migrate(app).await?;
        let rows: Vec<(i64, String)> =
            sqlx::query_as("SELECT id,images FROM posts WHERE source='' AND images LIKE '%wall:%'")
                .fetch_all(&app.db)
                .await?;
        for (id, images) in rows {
            let images: Vec<String> = serde_json::from_str(&images)?;
            if let Some(reference) = images.iter().find(|r| r.starts_with("wall:")) {
                sqlx::query("UPDATE posts SET source='wallpapers',source_key=? WHERE id=?")
                    .bind(reference)
                    .bind(id)
                    .execute(&app.db)
                    .await?;
            }
        }
        Ok(())
    }
    async fn upload(&self, app: &App, input: Value, files: Vec<media::Upload>) -> Result<Value> {
        if input["op"] != "upload" {
            return Err(Error::invalid("操作无效"));
        }
        uploads::upload(app, input, files).await
    }
    async fn admin(&self, app: &App, input: Value) -> Result<Value> {
        if input["op"] == "delete" {
            operations::delete(app, input).await
        } else {
            Err(Error::invalid("插件不支持此管理操作"))
        }
    }
    async fn data(
        &self,
        app: &App,
        _params: &HashMap<String, String>,
        _admin: bool,
    ) -> Result<Value> {
        let config = super::config(self, &Settings::load(app).await?);
        let items: Vec<Wall> = sqlx::query_as(
            "SELECT id,path,name,width,height,bytes,mtime FROM wallpapers ORDER BY mtime DESC,name",
        )
        .fetch_all(&app.db)
        .await?;
        let available = match root(app).await {
            Ok(root) => tokio::fs::read_dir(root).await.is_ok(),
            Err(_) => false,
        };
        Ok(
            json!({"items":items,"available":available,"batch":config.number("gallery_batch"),"interval":config.number("wallpaper_sync_interval")}),
        )
    }
    async fn resolve_ref(&self, app: &App, reference: &str) -> Result<Option<Value>> {
        let Some(id) = reference.strip_prefix("wall:") else {
            return Ok(None);
        };
        let wall: Option<Wall> = sqlx::query_as(
            "SELECT id,path,name,width,height,bytes,mtime FROM wallpapers WHERE id=?",
        )
        .bind(id)
        .fetch_optional(&app.db)
        .await?;
        Ok(wall.map(|w|json!({"ref":reference,"url":w.url(),"thumb":format!("{}&thumb=1",w.url()),"name":w.name,"width":w.width,"height":w.height,"bytes":w.bytes})))
    }
    async fn export_snapshot(&self, app: &App) -> Result<Option<super::ExportSnapshot>> {
        let settings = super::config(self, &Settings::load(app).await?);
        let items = sqlx::query(
            "SELECT id,name,width,height,bytes,mtime FROM wallpapers ORDER BY mtime DESC,name",
        )
        .fetch_all(&app.db)
        .await?
        .into_iter()
        .map(crate::api::row_json)
        .collect::<Vec<_>>();
        let references = items
            .iter()
            .map(|item| format!("wall:{}", item["id"].as_str().unwrap()))
            .collect();
        Ok(Some(super::ExportSnapshot {
            data: json!({"items":items,"batch":settings.number("gallery_batch")}),
            references,
        }))
    }
    async fn backup_paths(&self, app: &App) -> Result<Vec<(String, PathBuf)>> {
        let settings = Settings::load(app).await?;
        let directory = settings.get("wallpaper_dir");
        if directory.is_empty() {
            return Ok(Vec::new());
        }
        Ok(vec![("wallpaper_dir".into(), PathBuf::from(directory))])
    }
    fn export_data(
        &self,
        data: &Value,
        _posts: &[Value],
        media: &std::collections::BTreeMap<String, Value>,
    ) -> Value {
        let items: Vec<_> = data["items"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|item| {
                let descriptor = media.get(&format!("wall:{}", item["id"].as_str().unwrap()))?;
                let mut item = item.clone();
                item["original"] = descriptor["url"].clone();
                item["thumbnail"] = descriptor["thumb"].clone();
                for field in ["width", "height", "bytes"] {
                    item[field] = descriptor[field].clone();
                }
                Some(item)
            })
            .collect();
        json!({"items":items,"meta":{"batch":data["batch"]}})
    }
    async fn export_source(&self, app: &App, reference: &str) -> Result<Option<PathBuf>> {
        let Some(id) = reference.strip_prefix("wall:") else {
            return Ok(None);
        };
        let relative: Option<String> = sqlx::query_scalar("SELECT path FROM wallpapers WHERE id=?")
            .bind(id)
            .fetch_optional(&app.db)
            .await?;
        match relative {
            Some(relative) => Ok(Some(file(&root(app).await?, &relative).await?)),
            None => Ok(None),
        }
    }
    async fn image(&self, app: &App, params: &HashMap<String, String>) -> Result<Option<Response>> {
        let Some(relative) = params.get("file") else {
            return Ok(None);
        };
        let wall: Wall = sqlx::query_as(
            "SELECT id,path,name,width,height,bytes,mtime FROM wallpapers WHERE id=? AND path=?",
        )
        .bind(crate::auth::digest(relative))
        .bind(relative)
        .fetch_optional(&app.db)
        .await?
        .ok_or_else(|| Error::not_found("图片不存在"))?;
        let root = root(app).await?;
        let settings = Settings::load(app).await?;
        let source = file(&root, relative).await?;
        let cache = app
            .data
            .join("wallpaper-thumbs")
            .join(wall.cache(&root, &settings));
        let path = if params.get("thumb").is_some_and(|s| s == "1") && cache.is_file() {
            cache
        } else {
            source
        };
        use tokio::io::AsyncReadExt;
        let mut header = [0u8; 32];
        let mut input = tokio::fs::File::open(&path).await?;
        let count = input.read(&mut header).await?;
        let format =
            image::guess_format(&header[..count]).map_err(|_| Error::not_found("图片不存在"))?;
        let mut service = tower_http::services::ServeFile::new_with_mime(
            path,
            &format.to_mime_type().parse().unwrap(),
        );
        let response = service
            .try_call(axum::http::Request::new(axum::body::Body::empty()))
            .await
            .map_err(|e| Error::Internal(e.to_string()))?;
        let mut response = response.into_response();
        if params.get("download").is_some_and(|s| s == "1") {
            let encoded: String =
                url::form_urlencoded::byte_serialize(wall.name.as_bytes()).collect();
            response.headers_mut().insert(
                axum::http::header::CONTENT_DISPOSITION,
                format!("attachment; filename*=UTF-8''{encoded}")
                    .parse()
                    .unwrap(),
            );
        }
        Ok(Some(response))
    }
    async fn sync(&self, app: &App, ctx: &tasks::Context, _params: Value) -> anyhow::Result<Value> {
        {
            let _guard = app.content_lock.write().await;
            if operations::recover(app).await? {
                bail!("壁纸磁盘离线，等待恢复未完成的文件操作");
            }
        }
        let settings = Settings::load(app).await?;
        let config = super::config(self, &settings);
        let root = root(app).await?;
        let before = tokio::fs::metadata(&root).await?;
        let mut directories = vec![root.clone()];
        let mut walls = Vec::new();
        let cache = app.data.join("wallpaper-thumbs");
        tokio::fs::create_dir_all(&cache).await?;
        while let Some(directory) = directories.pop() {
            ctx.check()?;
            let mut entries = tokio::fs::read_dir(directory).await?;
            while let Some(entry) = entries.next_entry().await? {
                ctx.check()?;
                let kind = entry.file_type().await?;
                if kind.is_symlink() {
                    continue;
                }
                if kind.is_dir() {
                    directories.push(entry.path());
                    continue;
                }
                if !kind.is_file() || entry.file_name().to_string_lossy().starts_with('.') {
                    continue;
                }
                let path = entry.path();
                let relative = path
                    .strip_prefix(&root)?
                    .to_str()
                    .ok_or_else(|| anyhow::anyhow!("壁纸文件名必须为 UTF-8"))?
                    .to_owned();
                let source = file(&root, &relative).await?;
                let metadata = tokio::fs::metadata(&source).await?;
                let Ok((width, height, _)) = media::inspect(app, &source, &ctx.stop).await else {
                    ctx.check()?;
                    continue;
                };
                let wall = Wall {
                    id: crate::auth::digest(&relative),
                    name: entry.file_name().to_str().unwrap().into(),
                    path: relative,
                    width: width as i64,
                    height: height as i64,
                    bytes: metadata.len() as i64,
                    mtime: metadata.mtime(),
                };
                if !cache.join(wall.cache(&root, &settings)).is_file() {
                    let _permit = tokio::select! {_=ctx.stop.cancelled()=>bail!("任务已取消"),permit=app.media_slots.acquire()=>permit?};
                    media::thumbnail(
                        &source,
                        &ctx.directory.join(wall.cache(&root, &settings)),
                        settings.number("thumb_size"),
                        settings.number("thumb_quality"),
                        &ctx.stop,
                    )
                    .await?;
                }
                walls.push(wall);
                ctx.progress("扫描壁纸", walls.len() as i64, None).await?;
            }
        }
        walls.sort_by(|a, b| b.mtime.cmp(&a.mtime).then(a.name.cmp(&b.name)));
        let _guard = app.content_lock.write().await;
        ctx.check()?;
        let after = tokio::fs::metadata(&root).await?;
        if before.dev() != after.dev() || before.ino() != after.ino() {
            bail!("壁纸目录已改变，本次同步已取消");
        }
        let mut tx = app.db.begin_with("BEGIN IMMEDIATE").await?;
        let known: Vec<String> = sqlx::query_scalar("SELECT id FROM wallpapers")
            .fetch_all(&mut *tx)
            .await?;
        let present: HashSet<&str> = walls.iter().map(|w| w.id.as_str()).collect();
        let mut removed = 0;
        for id in known {
            if !present.contains(id.as_str()) {
                removed += forget(&mut tx, &id).await?.len();
            }
        }
        let template = config.get("wallpaper_post_template");
        let backfilled: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM settings WHERE key='plugin:wallpapers:backfilled')",
        )
        .fetch_one(&mut *tx)
        .await?;
        let mut added = 0;
        for wall in &walls {
            ctx.check()?;
            sqlx::query("INSERT INTO wallpapers(id,path,name,width,height,bytes,mtime) VALUES(?,?,?,?,?,?,?) ON CONFLICT(id) DO UPDATE SET path=excluded.path,name=excluded.name,width=excluded.width,height=excluded.height,bytes=excluded.bytes,mtime=excluded.mtime")
                .bind(&wall.id).bind(&wall.path).bind(&wall.name).bind(wall.width).bind(wall.height).bind(wall.bytes).bind(wall.mtime).execute(&mut *tx).await?;
            if !template.is_empty() {
                if backfilled {
                    let reference = format!("wall:{}", wall.id);
                    let date = DateTime::from_timestamp(wall.mtime, 0)
                        .unwrap_or_else(Utc::now)
                        .with_timezone(&app.config.timezone)
                        .to_rfc3339();
                    added+=sqlx::query("INSERT INTO posts(content,images,created_at,updated_at,source,source_key) SELECT ?,?,?,?,'wallpapers',? WHERE NOT EXISTS(SELECT 1 FROM wallpapers WHERE id=? AND post_id IS NOT NULL) AND NOT EXISTS(SELECT 1 FROM posts WHERE source='wallpapers' AND source_key=?) AND NOT EXISTS(SELECT 1 FROM plugin_tombstones WHERE plugin='wallpapers' AND key=?)")
                        .bind(template.replace("{name}",&wall.name)).bind(json!([reference]).to_string()).bind(&date).bind(&date).bind(&reference).bind(&wall.id).bind(&reference).bind(&reference).execute(&mut *tx).await?.rows_affected();
                }
                sqlx::query("UPDATE wallpapers SET post_id=1 WHERE id=?")
                    .bind(&wall.id)
                    .execute(&mut *tx)
                    .await?;
            }
        }
        if !template.is_empty() {
            sqlx::query("INSERT INTO settings(key,value) VALUES('plugin:wallpapers:backfilled','1') ON CONFLICT(key) DO NOTHING").execute(&mut *tx).await?;
        }
        ctx.check()?;
        tx.commit().await?;
        let names: HashSet<String> = walls
            .iter()
            .map(|wall| wall.cache(&root, &settings))
            .collect();
        for name in &names {
            let staged = ctx.directory.join(name);
            if staged.is_file() {
                tokio::fs::rename(staged, cache.join(name)).await?;
            }
        }
        let mut files = tokio::fs::read_dir(&cache).await?;
        while let Some(entry) = files.next_entry().await? {
            if !names.contains(entry.file_name().to_string_lossy().as_ref()) {
                media::remove_file(&entry.path()).await?;
            }
        }
        Ok(json!({"added":added,"removed":removed,"scanned":walls.len()}))
    }
}
