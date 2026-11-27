use super::*;
use tokio::io::AsyncWriteExt;

#[derive(FromRow)]
struct Pending {
    token: String,
    root: String,
    path: String,
    staged: String,
    cache: String,
    committed: bool,
    root_dev: i64,
    root_ino: i64,
}
pub async fn migrate(app: &App) -> Result<()> {
    sqlx::query("CREATE TABLE IF NOT EXISTS wallpaper_uploads(token TEXT PRIMARY KEY,root TEXT NOT NULL,path TEXT NOT NULL DEFAULT '',staged TEXT NOT NULL,cache TEXT NOT NULL DEFAULT '',committed INTEGER NOT NULL DEFAULT 0,root_dev INTEGER NOT NULL,root_ino INTEGER NOT NULL)").execute(&app.db).await?;
    Ok(())
}
pub async fn recover(app: &App) -> Result<bool> {
    let rows: Vec<Pending> = sqlx::query_as("SELECT * FROM wallpaper_uploads")
        .fetch_all(&app.db)
        .await?;
    let mut pending = false;
    for row in rows {
        let root = Path::new(&row.root);
        match tokio::fs::metadata(root).await {
            Ok(meta) if meta.dev() as i64 == row.root_dev && meta.ino() as i64 == row.root_ino => {}
            _ => {
                pending = true;
                continue;
            }
        }
        let staged = root.join(&row.staged);
        let thumb = root.join(format!("{}.webp", row.staged));
        if !row.committed && !row.path.is_empty() {
            let target = root.join(&row.path);
            let parent = tokio::fs::canonicalize(target.parent().unwrap()).await?;
            if !parent.starts_with(root) {
                return Err(Error::invalid("壁纸上传恢复路径已改变"));
            }
            if let Some(staged_meta) = metadata(&staged).await?
                && let Some(target_meta) = metadata(&target).await?
                && staged_meta.is_file()
                && target_meta.is_file()
                && staged_meta.dev() == target_meta.dev()
                && staged_meta.ino() == target_meta.ino()
            {
                media::remove_file(&target).await?;
            }
        }
        if row.committed && !row.cache.is_empty() && thumb.exists() {
            let cache = app.data.join("wallpaper-thumbs");
            tokio::fs::create_dir_all(&cache).await?;
            tokio::fs::rename(&thumb, cache.join(&row.cache)).await?;
        } else {
            media::remove_file(&thumb).await?;
        }
        media::remove_file(&staged).await?;
        sqlx::query("DELETE FROM wallpaper_uploads WHERE token=?")
            .bind(row.token)
            .execute(&app.db)
            .await?;
    }
    Ok(pending)
}

pub async fn upload(app: &App, input: Value, files: Vec<media::Upload>) -> Result<Value> {
    let _operation = super::super::mutation_guard("wallpapers")?;
    {
        let _guard = app.content_lock.write().await;
        if operations::recover(app).await? {
            return Err(Error::invalid("壁纸磁盘离线，等待恢复未完成的文件操作"));
        }
    }
    let count = input["count"]
        .as_u64()
        .or_else(|| input["count"].as_str().and_then(|s| s.parse().ok()));
    if files.is_empty()
        || count != Some(files.len() as u64)
        || files.iter().any(|f| f.field != "images[]")
    {
        return Err(Error::invalid("上传数量或图片字段无效"));
    }
    let root = root(app).await?;
    let root_meta = tokio::fs::metadata(&root).await?;
    let settings = Settings::load(app).await?;
    let config = super::super::config(&Wallpapers, &settings);
    if files
        .iter()
        .any(|f| f.bytes.len() > settings.number("upload_max_mb") as usize * 1024 * 1024)
    {
        return Err(Error::invalid("单张图片超过上传大小限制"));
    }
    let result=async {
        let mut prepared=Vec::new();
        for upload in files {
            let token=crate::auth::random_token()?;
            let staged=format!(".upload-{token}");
            sqlx::query("INSERT INTO wallpaper_uploads(token,root,staged,root_dev,root_ino) VALUES(?,?,?,?,?)")
                .bind(&token).bind(root.to_string_lossy().as_ref()).bind(&staged).bind(root_meta.dev() as i64).bind(root_meta.ino() as i64).execute(&app.db).await?;
            let source=root.join(&staged);
            let mut output=tokio::fs::OpenOptions::new().write(true).create_new(true).open(&source).await?;
            output.write_all(&upload.bytes).await?;
            drop(output);
            let (width,height,extension)=media::inspect(app,&source,&app.shutdown).await?;
            let directory=if width>height {root.clone()} else {root.join("Portrait")};
            tokio::fs::create_dir_all(&directory).await?;
            let directory=tokio::fs::canonicalize(directory).await?;
            if !directory.starts_with(&root){return Err(Error::invalid("上传目录必须位于壁纸目录内"));}
            let stem=Path::new(&upload.name).file_stem().and_then(|s|s.to_str()).unwrap_or("wallpaper");
            let stem:String=stem.chars().map(|c|if c.is_control() || "/\\:*?\"<>|".contains(c){'_'}else{c}).collect();
            let stem:String=stem.trim_matches(['.',' ']).chars().take(80).collect();
            let stem=if stem.is_empty(){"wallpaper"}else{&stem};
            let mut name=format!("{stem}.{extension}");
            if directory.join(&name).exists() || prepared.iter().any(|(_,_,w):&(String,String,Wall)|w.path==directory.join(&name).strip_prefix(&root).unwrap().to_string_lossy()) {
                name=format!("{stem}-{}.{extension}",&token[..16]);
            }
            let path=directory.join(&name).strip_prefix(&root).unwrap().to_str().unwrap().to_owned();
            let metadata=tokio::fs::metadata(&source).await?;
            let wall=Wall{id:crate::auth::digest(&path),path,name,width:width as i64,height:height as i64,bytes:metadata.len() as i64,mtime:metadata.mtime()};
            let cache=wall.cache(&root,&settings);
            sqlx::query("UPDATE wallpaper_uploads SET path=?,cache=? WHERE token=?").bind(&wall.path).bind(&cache).bind(&token).execute(&app.db).await?;
            {
                let _permit=app.media_slots.acquire().await.map_err(|e|Error::Internal(e.to_string()))?;
                media::thumbnail(&source,&root.join(format!("{staged}.webp")),settings.number("thumb_size"),settings.number("thumb_quality"),&app.shutdown).await.map_err(|e|Error::Internal(e.to_string()))?;
            }
            prepared.push((token,staged,wall));
        }
        let _guard=app.content_lock.write().await;
        let current=tokio::fs::metadata(&root).await?;
        if current.dev()!=root_meta.dev() || current.ino()!=root_meta.ino(){return Err(Error::invalid("壁纸目录已改变"));}
        let mut tx=app.db.begin_with("BEGIN IMMEDIATE").await?;
        let mut refs=Vec::new();
        let mut uploaded=Vec::new();
        for (token,staged,wall) in &prepared {
            let target=root.join(&wall.path);
            let parent=tokio::fs::canonicalize(target.parent().unwrap()).await?;
            if !parent.starts_with(&root) || parent!=target.parent().unwrap(){return Err(Error::invalid("上传目录已改变"));}
            tokio::fs::hard_link(root.join(staged),target).await?;
            sqlx::query("INSERT INTO wallpapers(id,path,name,width,height,bytes,mtime,post_id) VALUES(?,?,?,?,?,?,?,1)")
                .bind(&wall.id).bind(&wall.path).bind(&wall.name).bind(wall.width).bind(wall.height).bind(wall.bytes).bind(wall.mtime).execute(&mut *tx).await?;
            sqlx::query("UPDATE wallpaper_uploads SET committed=1 WHERE token=?").bind(token).execute(&mut *tx).await?;
            refs.push(format!("wall:{}",wall.id));uploaded.push(wall.path.clone());
        }
        let template=config.get("wallpaper_post_template");
        if !template.is_empty() {
            let content=if prepared.len()==1 {template.replace("{name}",&prepared[0].2.name)}else{format!("收了 {} 张新壁纸",prepared.len())};
            let now=app.config.now();
            sqlx::query("INSERT INTO posts(content,images,created_at,updated_at,source,source_key) VALUES(?,?,?,?,'wallpapers',?)")
                .bind(content).bind(json!(refs).to_string()).bind(&now).bind(&now).bind(format!("wall-upload:{}",crate::auth::random_token()?)).execute(&mut *tx).await?;
        }
        tx.commit().await?;
        Ok::<_,Error>(json!({"uploaded":uploaded}))
    }.await;
    let _guard = app.content_lock.write().await;
    let cleanup = recover(app).await;
    match result {
        Ok(value) => {
            if let Err(error) = cleanup {
                tracing::warn!(error=%error,"壁纸上传已提交，缩略图及暂存文件等待恢复");
            }
            Ok(value)
        }
        Err(error) => {
            cleanup?;
            Err(error)
        }
    }
}

async fn metadata(path: &Path) -> Result<Option<std::fs::Metadata>> {
    match tokio::fs::symlink_metadata(path).await {
        Ok(metadata) => Ok(Some(metadata)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}
