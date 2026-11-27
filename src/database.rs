use anyhow::{Result, bail};
use sqlx::{
    SqlitePool,
    sqlite::{SqliteConnectOptions, SqlitePoolOptions},
};
use std::{path::Path, time::Duration};

pub async fn open(data: &Path) -> Result<SqlitePool> {
    tokio::fs::create_dir_all(data.join("uploads")).await?;
    let pool = SqlitePoolOptions::new()
        .max_connections(4)
        .connect_with(
            SqliteConnectOptions::new()
                .filename(data.join("blog.sqlite"))
                .create_if_missing(true)
                .busy_timeout(Duration::from_secs(5)),
        )
        .await?;
    let legacy: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='schema_versions')",
    )
    .fetch_one(&pool)
    .await?;
    if legacy {
        let version: Option<i64> =
            sqlx::query_scalar("SELECT version FROM schema_versions WHERE component='core'")
                .fetch_optional(&pool)
                .await?;
        if version != Some(4) {
            bail!("不支持此旧数据库版本；导入数据的核心 schema_versions 版本应为 4");
        }
    }
    sqlx::migrate!("./migrations").run(&pool).await?;
    Ok(pool)
}
