use super::*;

#[derive(FromRow)]
struct Deletion {
    id: String,
    root: String,
    path: String,
    staged: String,
    root_dev: i64,
    root_ino: i64,
    file_dev: i64,
    file_ino: i64,
}

pub async fn migrate(app: &App) -> Result<()> {
    sqlx::query("CREATE TABLE IF NOT EXISTS wallpaper_deletions(id TEXT PRIMARY KEY,root TEXT NOT NULL,path TEXT NOT NULL,staged TEXT NOT NULL,root_dev INTEGER NOT NULL,root_ino INTEGER NOT NULL,file_dev INTEGER NOT NULL,file_ino INTEGER NOT NULL)").execute(&app.db).await?;
    super::uploads::migrate(app).await?;
    recover(app).await?;
    Ok(())
}

// Keep the journal when a source disk is offline; retry before the next mutation.
pub async fn recover(app: &App) -> Result<bool> {
    let entries: Vec<Deletion> = sqlx::query_as("SELECT * FROM wallpaper_deletions")
        .fetch_all(&app.db)
        .await?;
    let mut pending = false;
    for entry in entries {
        let root = Path::new(&entry.root);
        let metadata = match tokio::fs::metadata(root).await {
            Ok(metadata)
                if metadata.dev() as i64 == entry.root_dev
                    && metadata.ino() as i64 == entry.root_ino =>
            {
                metadata
            }
            _ => {
                pending = true;
                continue;
            }
        };
        if !metadata.is_dir() {
            pending = true;
            continue;
        }
        let source = root.join(&entry.path);
        let staged = root.join(&entry.staged);
        let parent = match tokio::fs::canonicalize(staged.parent().unwrap()).await {
            Ok(parent) => parent,
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::NotFound | std::io::ErrorKind::PermissionDenied
                ) =>
            {
                pending = true;
                continue;
            }
            Err(error) => return Err(error.into()),
        };
        if !parent.starts_with(root) || source.parent() != staged.parent() {
            return Err(Error::invalid("壁纸恢复路径已改变"));
        }
        let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM wallpapers WHERE id=?)")
            .bind(&entry.id)
            .fetch_one(&app.db)
            .await?;
        match tokio::fs::symlink_metadata(&staged).await {
            Ok(metadata) => {
                if !metadata.is_file()
                    || metadata.dev() as i64 != entry.file_dev
                    || metadata.ino() as i64 != entry.file_ino
                {
                    return Err(Error::invalid("壁纸恢复文件已改变"));
                }
                if exists {
                    match tokio::fs::hard_link(&staged, &source).await {
                        Ok(()) => {}
                        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                            let source = tokio::fs::symlink_metadata(&source).await?;
                            if !source.is_file()
                                || source.dev() != metadata.dev()
                                || source.ino() != metadata.ino()
                            {
                                return Err(Error::invalid(
                                    "壁纸恢复位置已有其他文件，请先处理冲突",
                                ));
                            }
                        }
                        Err(error) => return Err(error.into()),
                    }
                }
                media::remove_file(&staged).await?;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        if !exists {
            clear_cache(app, &entry.id).await?;
        }
        sqlx::query("DELETE FROM wallpaper_deletions WHERE id=?")
            .bind(&entry.id)
            .execute(&app.db)
            .await?;
    }
    Ok(super::uploads::recover(app).await? || pending)
}

pub async fn delete(app: &App, input: Value) -> Result<Value> {
    let _operation = super::super::mutation_guard("wallpapers")?;
    let _content = app.content_lock.write().await;
    if recover(app).await? {
        return Err(Error::invalid("壁纸磁盘离线，等待恢复未完成的文件操作"));
    }
    let id = input["wall_id"].as_str().unwrap_or("");
    if id.len() != 64
        || !id
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
    {
        return Err(Error::invalid("壁纸标识无效"));
    }
    let wall: Wall =
        sqlx::query_as("SELECT id,path,name,width,height,bytes,mtime FROM wallpapers WHERE id=?")
            .bind(id)
            .fetch_optional(&app.db)
            .await?
            .ok_or_else(|| Error::not_found("壁纸不存在"))?;
    let root = root(app).await?;
    let source = root.join(&wall.path);
    let original = match tokio::fs::symlink_metadata(&source).await {
        Ok(metadata) => Some(metadata),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(error.into()),
    };
    if let Some(metadata) = original {
        let canonical = file(&root, &wall.path).await?;
        if !metadata.is_file() || canonical != source {
            return Err(Error::invalid("壁纸路径已改变"));
        }
        let root_meta = tokio::fs::metadata(&root).await?;
        let staged = source.with_file_name(format!(".deleted-{}", crate::auth::random_token()?));
        sqlx::query("INSERT INTO wallpaper_deletions(id,root,path,staged,root_dev,root_ino,file_dev,file_ino) VALUES(?,?,?,?,?,?,?,?)")
            .bind(id).bind(root.to_string_lossy().as_ref()).bind(&wall.path).bind(staged.strip_prefix(&root).unwrap().to_string_lossy().as_ref())
            .bind(root_meta.dev() as i64).bind(root_meta.ino() as i64).bind(metadata.dev() as i64).bind(metadata.ino() as i64).execute(&app.db).await?;
        if let Err(error) = tokio::fs::rename(&source, &staged).await {
            recover(app).await?;
            return Err(error.into());
        }
    }
    let result = async {
        let mut tx = app.db.begin_with("BEGIN IMMEDIATE").await?;
        let deleted = forget(&mut tx, id).await?;
        tx.commit().await?;
        Ok::<_, Error>(deleted)
    }
    .await;
    recover(app).await?;
    let deleted = result?;
    clear_cache(app, id).await?;
    Ok(json!({"deleted":wall.name,"deleted_posts":deleted}))
}

async fn clear_cache(app: &App, id: &str) -> Result<()> {
    let cache = app.data.join("wallpaper-thumbs");
    if let Ok(mut files) = tokio::fs::read_dir(cache).await {
        while let Some(entry) = files.next_entry().await? {
            if entry
                .file_name()
                .to_string_lossy()
                .starts_with(&format!("{id}-"))
            {
                media::remove_file(&entry.path()).await?;
            }
        }
    }
    Ok(())
}
