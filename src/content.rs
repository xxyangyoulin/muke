use crate::{
    api::App,
    config::Settings,
    error::{Error, Result},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::{FromRow, Sqlite, Transaction};
use std::sync::LazyLock;

#[derive(Debug, Deserialize, Serialize, FromRow)]
pub struct Article {
    pub id: i64,
    pub title: String,
    pub content: String,
    pub format: String,
    pub images: String,
    pub tags: String,
    pub draft: i64,
    pub created_at: String,
    pub updated_at: String,
}
#[derive(Deserialize)]
pub struct SaveArticle {
    #[serde(default)]
    pub id: Option<i64>,
    pub title: String,
    pub content: String,
    #[serde(default)]
    pub format: String,
    #[serde(default)]
    pub tags: String,
    #[serde(default)]
    pub draft: i64,
}
impl SaveArticle {
    fn normalize(&mut self, settings: &Settings) -> Result<()> {
        self.title = self.title.trim().to_owned();
        if !(1..=80).contains(&self.title.chars().count()) {
            return Err(Error::invalid("标题需 1–80 字"));
        }
        if self.content.trim().is_empty()
            || self.content.len() > settings.number("article_max_bytes") as usize
        {
            return Err(Error::invalid("正文不能为空，且不超过大小上限"));
        }
        self.format = if self.format == "html" { "html" } else { "md" }.into();
        self.draft = i64::from(self.draft != 0);
        let mut tags = Vec::new();
        for tag in self
            .tags
            .split(|c: char| c == ',' || c == '，' || c == '、' || c.is_whitespace())
            .filter(|s| !s.is_empty() && s.chars().count() <= 20)
            .take(5)
        {
            if !tags.contains(&tag) {
                tags.push(tag);
            }
        }
        self.tags = tags.join(",");
        Ok(())
    }
}
pub fn image_refs(content: &str) -> Vec<String> {
    static RE: LazyLock<regex::Regex> = LazyLock::new(|| {
        regex::Regex::new(
            r"(?:/image\?upload=|/media/uploads/)([a-f0-9]{32}\.(?:jpg|png|webp|gif|avif))",
        )
        .unwrap()
    });
    let mut refs = Vec::new();
    for m in RE.captures_iter(content) {
        let name = m[1].to_owned();
        if !refs.contains(&name) {
            refs.push(name);
        }
    }
    refs
}
pub async fn save_article(app: &App, mut input: SaveArticle) -> Result<Value> {
    input.normalize(&Settings::load(app).await?)?;
    let _guard = app.content_lock.write().await;
    let mut tx = app.db.begin_with("BEGIN IMMEDIATE").await?;
    let now = app.config.now();
    if let Some(id) = input.id.filter(|id| *id != 0) {
        let existing = sqlx::query_as::<_, Article>("SELECT * FROM articles WHERE id=?")
            .bind(id)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or_else(|| Error::not_found("文章已不存在"))?;
        input.format = existing.format.clone();
        update_revision(&mut tx, &existing, &input, &now).await?;
    } else {
        sqlx::query("INSERT INTO articles(title,content,format,images,tags,draft,created_at,updated_at) VALUES(?,?,?,?,?,?,?,?)")
            .bind(&input.title).bind(&input.content).bind(&input.format).bind(json!(image_refs(&input.content)).to_string()).bind(&input.tags).bind(input.draft).bind(&now).bind(&now).execute(&mut *tx).await?;
    }
    tx.commit().await?;
    Ok(json!({"ok":true}))
}
async fn update_revision(
    tx: &mut Transaction<'_, Sqlite>,
    existing: &Article,
    next: &SaveArticle,
    now: &str,
) -> Result<()> {
    let images = json!(image_refs(&next.content)).to_string();
    if existing.title == next.title
        && existing.content == next.content
        && existing.format == next.format
        && existing.tags == next.tags
        && existing.draft == next.draft
        && serde_json::from_str::<Value>(&existing.images).ok()
            == serde_json::from_str::<Value>(&images).ok()
    {
        return Ok(());
    }
    sqlx::query("INSERT INTO article_revisions(article_id,title,content,format,tags,draft,images,updated_at,saved_at) VALUES(?,?,?,?,?,?,?,?,?)")
        .bind(existing.id).bind(&existing.title).bind(&existing.content).bind(&existing.format).bind(&existing.tags).bind(existing.draft).bind(&existing.images).bind(&existing.updated_at).bind(now).execute(&mut **tx).await?;
    sqlx::query("UPDATE articles SET title=?,content=?,format=?,tags=?,draft=?,images=?,updated_at=? WHERE id=?")
        .bind(&next.title).bind(&next.content).bind(&next.format).bind(&next.tags).bind(next.draft).bind(images).bind(now).bind(existing.id).execute(&mut **tx).await?;
    Ok(())
}
pub async fn restore_revision(app: &App, id: i64, revision: i64) -> Result<Value> {
    let _guard = app.content_lock.write().await;
    let mut tx = app.db.begin_with("BEGIN IMMEDIATE").await?;
    let existing = sqlx::query_as::<_, Article>("SELECT * FROM articles WHERE id=?")
        .bind(id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(|| Error::not_found("文章或历史版本已不存在"))?;
    let next=sqlx::query_as::<_,Article>("SELECT id,title,content,format,images,tags,draft,saved_at AS created_at,updated_at FROM article_revisions WHERE article_id=? AND id=?").bind(id).bind(revision).fetch_optional(&mut *tx).await?.ok_or_else(||Error::not_found("文章或历史版本已不存在"))?;
    let input = SaveArticle {
        id: Some(id),
        title: next.title,
        content: next.content,
        format: next.format,
        tags: next.tags,
        draft: next.draft,
    };
    update_revision(&mut tx, &existing, &input, &app.config.now()).await?;
    let restored = sqlx::query_as::<_, Article>("SELECT * FROM articles WHERE id=?")
        .bind(id)
        .fetch_one(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(json!({"ok":true,"article":restored}))
}
pub async fn revisions(app: &App, id: i64, revision: Option<i64>, cursor: i64) -> Result<Value> {
    let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM articles WHERE id=?)")
        .bind(id)
        .fetch_one(&app.db)
        .await?;
    if !exists {
        return Err(Error::not_found("文章已不存在"));
    }
    if let Some(revision) = revision {
        let row = sqlx::query("SELECT * FROM article_revisions WHERE article_id=? AND id=?")
            .bind(id)
            .bind(revision)
            .fetch_optional(&app.db)
            .await?
            .ok_or_else(|| Error::not_found("历史版本不存在"))?;
        return Ok(json!({"revision":crate::api::row_json(row)}));
    }
    let mut rows:Vec<Value>=sqlx::query("SELECT id,title,draft,updated_at,saved_at FROM article_revisions WHERE article_id=? AND (?=0 OR id<?) ORDER BY id DESC LIMIT 21").bind(id).bind(cursor).bind(cursor).fetch_all(&app.db).await?.into_iter().map(crate::api::row_json).collect();
    let more = rows.len() > 20;
    rows.truncate(20);
    let next = if more {
        rows.last().unwrap()["id"].clone()
    } else {
        Value::Null
    };
    Ok(json!({"revisions":rows,"next_cursor":next}))
}
pub async fn delete_article(app: &App, id: i64) -> Result<Value> {
    let _guard = app.content_lock.write().await;
    let mut tx = app.db.begin_with("BEGIN IMMEDIATE").await?;
    let images:Vec<String>=sqlx::query_scalar("SELECT images FROM articles WHERE id=? UNION ALL SELECT images FROM article_revisions WHERE article_id=?").bind(id).bind(id).fetch_all(&mut *tx).await?;
    sqlx::query("DELETE FROM article_revisions WHERE article_id=?")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM articles WHERE id=?")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM comments WHERE kind='article' AND target_id=?")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    for encoded in images {
        for name in serde_json::from_str::<Vec<String>>(&encoded).unwrap_or_default() {
            crate::media::delete_unused(app, &format!("up:{name}")).await?;
        }
    }
    Ok(json!({"ok":true}))
}
#[derive(Deserialize, Default)]
pub struct SavePost {
    #[serde(default)]
    pub id: i64,
    #[serde(default)]
    pub content: String,
    #[serde(default)]
    pub keep: Vec<String>,
}
pub async fn save_post(app: &App, mut input: SavePost, uploaded: Vec<String>) -> Result<Value> {
    let settings = Settings::load(app).await?;
    input.content = input.content.trim().to_owned();
    if input.content.chars().count() > settings.number("post_max_chars") as usize {
        return Err(Error::invalid(format!(
            "动态内容最多 {} 字",
            settings.number("post_max_chars")
        )));
    }
    let _guard = app.content_lock.write().await;
    let mut tx = app.db.begin_with("BEGIN IMMEDIATE").await?;
    let mut removed = Vec::new();
    let mut images = Vec::new();
    if input.id != 0 {
        let encoded: Option<String> = sqlx::query_scalar("SELECT images FROM posts WHERE id=?")
            .bind(input.id)
            .fetch_optional(&mut *tx)
            .await?;
        let encoded = encoded.ok_or_else(|| Error::not_found("动态已不存在"))?;
        for reference in serde_json::from_str::<Vec<String>>(&encoded).unwrap_or_default() {
            if input.keep.contains(&reference) {
                images.push(reference);
            } else {
                removed.push(reference);
            }
        }
    }
    images.extend(uploaded);
    if images.len() > settings.number("post_max_images") as usize {
        return Err(Error::invalid(format!(
            "每条动态最多 {} 张图片",
            settings.number("post_max_images")
        )));
    }
    if input.content.is_empty() && images.is_empty() {
        return Err(Error::invalid("写点什么，或者配一张图"));
    }
    let now = app.config.now();
    let encoded = json!(images).to_string();
    if input.id != 0 {
        sqlx::query("UPDATE posts SET content=?,images=?,updated_at=? WHERE id=?")
            .bind(input.content)
            .bind(encoded)
            .bind(now)
            .bind(input.id)
            .execute(&mut *tx)
            .await?;
    } else {
        sqlx::query("INSERT INTO posts(content,images,created_at,updated_at) VALUES(?,?,?,?)")
            .bind(input.content)
            .bind(encoded)
            .bind(&now)
            .bind(&now)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    for reference in removed {
        crate::media::delete_unused(app, &reference).await?;
    }
    if input.id == 0 {
        // 新动态触碰自动部署防抖（编辑不触发）；失败不阻塞发布
        if let Err(error) = crate::deploy::touch(app).await {
            tracing::warn!(%error, "自动部署触碰失败");
        }
    }
    Ok(json!({"ok":true}))
}
pub async fn delete_post(app: &App, id: i64) -> Result<Value> {
    let _guard = app.content_lock.write().await;
    let mut tx = app.db.begin_with("BEGIN IMMEDIATE").await?;
    let post: Option<(String, String, String, String)> =
        sqlx::query_as("SELECT images,source,source_key,meta FROM posts WHERE id=?")
            .bind(id)
            .fetch_optional(&mut *tx)
            .await?;
    if let Some((_, source, key, _)) = &post
        && !key.is_empty()
    {
        sqlx::query("INSERT OR IGNORE INTO plugin_tombstones(key,plugin,deleted_at) VALUES(?,?,?)")
            .bind(key)
            .bind(source)
            .bind(app.config.now())
            .execute(&mut *tx)
            .await?;
    }
    sqlx::query("DELETE FROM posts WHERE id=?")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM comments WHERE kind='post' AND target_id=?")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM post_likes WHERE post_id=?")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    if let Some((images, source, _, meta)) = post {
        let mut refs = serde_json::from_str::<Vec<String>>(&images).unwrap_or_default();
        refs.extend(crate::plugins::post_refs(&source, &meta));
        for reference in refs {
            crate::media::delete_unused(app, &reference).await?;
        }
    }
    Ok(json!({"ok":true}))
}
pub async fn pin_post(app: &App, id: i64, pinned: i64) -> Result<Value> {
    if id <= 0 || !matches!(pinned, 0 | 1) {
        return Err(Error::invalid("置顶参数无效"));
    }
    let _guard = app.content_lock.write().await;
    let result = sqlx::query("UPDATE posts SET pinned=? WHERE id=?")
        .bind(pinned)
        .bind(id)
        .execute(&app.db)
        .await?;
    if result.rows_affected() == 0 {
        return Err(Error::not_found("动态不存在"));
    }
    Ok(json!({"ok":true}))
}
