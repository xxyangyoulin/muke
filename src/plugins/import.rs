use crate::{api::App, media, tasks};
use anyhow::{Result, bail};
use serde_json::{Value, json};

pub struct Post {
    pub key: String,
    pub content: String,
    pub created_at: String,
    pub link: String,
    pub meta: Value,
    pub images: Vec<String>,
}

pub async fn recover(app: &App) -> Result<()> {
    sqlx::query("CREATE TABLE IF NOT EXISTS plugin_pending_media(reference TEXT PRIMARY KEY)")
        .execute(&app.db)
        .await?;
    let pending: Vec<String> = sqlx::query_scalar("SELECT reference FROM plugin_pending_media")
        .fetch_all(&app.db)
        .await?;
    for reference in pending {
        media::delete_unused(app, &reference).await?;
    }
    sqlx::query("DELETE FROM plugin_pending_media")
        .execute(&app.db)
        .await?;
    Ok(())
}

pub async fn known(app: &App, source: &str, key: &str) -> Result<bool> {
    Ok(sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM posts WHERE source=? AND source_key=? UNION ALL SELECT 1 FROM plugin_tombstones WHERE plugin=? AND key=?)")
        .bind(source).bind(key).bind(source).bind(key).fetch_one(&app.db).await?)
}

pub async fn save(app: &App, ctx: &tasks::Context, source: &str, post: Post) -> Result<u64> {
    let refs: Vec<String> = post
        .images
        .iter()
        .map(|name| format!("up:{name}"))
        .collect();
    let _guard = app.content_lock.write().await;
    ctx.check()?;
    for reference in &refs {
        sqlx::query("INSERT INTO plugin_pending_media(reference) VALUES(?)")
            .bind(reference)
            .execute(&app.db)
            .await?;
    }
    let result = async {
        for name in &post.images {
            ctx.check()?;
            tokio::fs::copy(ctx.directory.join(name), app.data.join("uploads").join(name)).await?;
        }
        let mut tx = app.db.begin_with("BEGIN IMMEDIATE").await?;
        ctx.check()?;
        let inserted = sqlx::query("INSERT INTO posts(content,created_at,updated_at,images,meta,source,source_key,link) SELECT ?,?,?,?,?,?,?,? WHERE NOT EXISTS(SELECT 1 FROM posts WHERE source=? AND source_key=?) AND NOT EXISTS(SELECT 1 FROM plugin_tombstones WHERE plugin=? AND key=?)")
            .bind(&post.content).bind(&post.created_at).bind(&post.created_at).bind(json!(refs).to_string()).bind(post.meta.to_string()).bind(source).bind(&post.key).bind(&post.link).bind(source).bind(&post.key).bind(source).bind(&post.key).execute(&mut *tx).await?.rows_affected();
        for reference in &refs {
            sqlx::query("DELETE FROM plugin_pending_media WHERE reference=?").bind(reference).execute(&mut *tx).await?;
        }
        tx.commit().await?;
        Ok::<_, anyhow::Error>(inserted)
    }.await;
    if !matches!(&result, Ok(n) if *n > 0) {
        for reference in &refs {
            media::delete_unused(app, reference).await?;
            sqlx::query("DELETE FROM plugin_pending_media WHERE reference=?")
                .bind(reference)
                .execute(&app.db)
                .await?;
        }
    }
    for name in &post.images {
        media::remove_file(&ctx.directory.join(name)).await?;
    }
    result
}

pub async fn fetch_image(
    app: &App,
    ctx: &tasks::Context,
    url: &str,
    min_size: u32,
    referer: &str,
) -> Result<String> {
    let bytes = app
        .network
        .get_with_referer(url, false, 15 * 1024 * 1024, &ctx.stop, Some(referer))
        .await?;
    let name = crate::auth::random_token()?[..32].to_owned();
    let source = ctx.directory.join(&name);
    tokio::fs::write(&source, bytes).await?;
    let result = async {
        let (width, height, ext) = media::inspect(app, &source, &ctx.stop).await?;
        if width < min_size || height < min_size {
            bail!("忽略小图片");
        }
        let name = format!("{name}.{ext}");
        tokio::fs::rename(&source, ctx.directory.join(&name)).await?;
        Ok(name)
    }
    .await;
    if result.is_err() {
        media::remove_file(&source).await?;
    }
    result
}
