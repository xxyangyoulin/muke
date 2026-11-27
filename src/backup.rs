use crate::api::App;
use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use std::{
    io::Write,
    path::{Path, PathBuf},
};
use tokio_util::sync::CancellationToken;
use zip::write::SimpleFileOptions;

/// 创建完整备份：数据库快照、上传图片、头像/封面与插件声明的资源目录。
/// 写入 archive（不能已存在）；失败时删除半成品。
pub async fn create(app: &App, archive: &Path, stop: &CancellationToken) -> Result<()> {
    // Keep the database snapshot and its media under the same mutation barrier.
    let _guard = tokio::select! {
        biased;
        _ = stop.cancelled() => bail!("任务已取消"),
        guard = app.content_lock.write() => guard,
    };
    if archive.exists() {
        bail!("备份目标已存在");
    }
    if let Some(parent) = archive.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let partial = archive.with_extension("zip.partial");
    let snapshot = archive.with_extension("sqlite.snapshot");
    let result = async {
        if stop.is_cancelled() {
            bail!("任务已取消");
        }
        // 数据库一致性快照
        sqlx::query("VACUUM INTO ?")
            .bind(snapshot.to_str().context("备份路径必须是 UTF-8")?)
            .execute(&app.db)
            .await?;
        let mut paths: Vec<(String, PathBuf)> = vec![("uploads".into(), app.data.join("uploads"))];
        for name in ["avatar.webp", "cover.webp"] {
            let path = app.data.join(name);
            if tokio::fs::try_exists(&path).await? {
                paths.push((name.into(), path));
            }
        }
        let mut manifest = json!({"app":"mkblog","version":1,"paths":{}});
        for (id, key, directory) in crate::plugins::backup_paths(app).await? {
            if !tokio::fs::symlink_metadata(&directory)
                .await
                .map(|metadata| metadata.is_dir())
                .unwrap_or(false)
            {
                bail!("插件 {id} 的资源目录不可读取");
            }
            let relative = format!("plugins/{id}/{key}");
            if let Some(parent) = archive.parent() {
                let absolute = std::fs::canonicalize(&directory)?;
                let target = parent.canonicalize()?;
                if target.starts_with(&absolute) {
                    bail!("备份目标不能位于被备份的资源目录内");
                }
            }
            paths.push((relative.clone(), directory));
            manifest["paths"][key] = json!(relative);
        }
        zip_entries(&partial, &paths, &snapshot, &manifest, stop).await?;
        if stop.is_cancelled() {
            bail!("任务已取消");
        }
        tokio::fs::rename(&partial, archive).await?;
        Ok(())
    }
    .await;
    if result.is_err() {
        let _ = tokio::fs::remove_file(&partial).await;
    }
    let _ = tokio::fs::remove_file(&snapshot).await;
    result
}

async fn zip_entries(
    target: &Path,
    paths: &[(String, PathBuf)],
    snapshot: &Path,
    manifest: &Value,
    stop: &CancellationToken,
) -> Result<()> {
    let entries: Vec<(String, PathBuf)> =
        std::iter::once(("blog.sqlite".to_owned(), snapshot.to_owned()))
            .chain(paths.iter().cloned())
            .collect();
    let target = target.to_owned();
    let manifest = manifest.to_string();
    let stop = stop.clone();
    tokio::task::spawn_blocking(move || {
        let file = std::fs::File::create(&target)?;
        let mut writer = zip::ZipWriter::new(file);
        let options = SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated)
            .unix_permissions(0o600);
        for (relative, source) in &entries {
            if stop.is_cancelled() {
                bail!("任务已取消");
            }
            let metadata = std::fs::symlink_metadata(source)?;
            if metadata.is_file() {
                writer.start_file(relative, options)?;
                let mut input = std::fs::File::open(source)?;
                std::io::copy(&mut input, &mut writer)?;
                writer.flush()?;
            } else if metadata.is_dir() {
                writer.add_directory(relative.clone(), options)?;
                let mut directories = vec![source.clone()];
                while let Some(directory) = directories.pop() {
                    for entry in std::fs::read_dir(&directory)? {
                        if stop.is_cancelled() {
                            bail!("任务已取消");
                        }
                        let entry = entry?;
                        let path = entry.path();
                        let entry_metadata = entry.metadata()?;
                        let name = format!(
                            "{}/{}",
                            relative,
                            path.strip_prefix(source)
                                .unwrap()
                                .to_string_lossy()
                                .replace('\\', "/")
                        );
                        if entry_metadata.is_dir() {
                            writer.add_directory(name, options)?;
                            directories.push(path);
                        } else if entry_metadata.is_file() {
                            writer.start_file(name, options)?;
                            let mut input = std::fs::File::open(&path)?;
                            std::io::copy(&mut input, &mut writer)?;
                            writer.flush()?;
                        } else {
                            bail!("备份资源不能包含符号链接");
                        }
                    }
                }
            } else {
                bail!("备份资源不能包含符号链接");
            }
        }
        writer.start_file("manifest.json", options)?;
        writer.write_all(manifest.as_bytes())?;
        let file = writer.finish()?;
        file.sync_all()?;
        Ok(())
    })
    .await?
}

/// 恢复备份到已存在的空目录：校验清单、路径与符号链接后解压，检查数据库完整性，
/// 并把插件资源目录写入恢复库的设置表。
pub async fn restore(archive: &Path, target: &Path) -> Result<()> {
    let checks = tokio::task::spawn_blocking({
        let archive = archive.to_owned();
        let target = target.to_owned();
        move || -> Result<Value> {
            if !std::fs::symlink_metadata(&target)
                .map(|metadata| metadata.is_dir())
                .unwrap_or(false)
                || std::fs::read_dir(&target)?.next().is_some()
            {
                bail!("恢复目标必须是已存在的空目录");
            }
            let file = std::fs::File::open(&archive).context("无法打开备份")?;
            let mut zip = zip::ZipArchive::new(file)?;
            let manifest: Value = {
                let mut entry = zip.by_name("manifest.json").context("备份格式不正确")?;
                let mut content = Vec::new();
                std::io::Read::read_to_end(&mut entry, &mut content)?;
                serde_json::from_slice(&content).context("备份格式不正确")?
            };
            if manifest["app"] != "mkblog"
                || manifest["version"] != 1
                || zip.by_name("blog.sqlite").is_err()
            {
                bail!("备份格式不正确");
            }
            for index in 0..zip.len() {
                let entry = zip.by_index(index)?;
                let name = entry.name().context("备份条目名无效")?;
                if name.starts_with('/')
                    || name.contains('\\')
                    || name.contains('\0')
                    || name.split('/').any(|segment| segment == "..")
                    || entry.is_symlink()
                {
                    bail!("备份包含非法路径");
                }
            }
            let Some(paths) = manifest["paths"].as_object() else {
                bail!("备份格式不正确");
            };
            for (key, relative) in paths {
                let relative = relative.as_str().unwrap_or("");
                let parts: Vec<&str> = relative.split('/').collect();
                let valid = parts.len() == 3
                    && parts[0] == "plugins"
                    && parts[1..3].iter().all(|part| {
                        !part.is_empty()
                            && part
                                .chars()
                                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
                    })
                    && !key.is_empty()
                    && key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
                if !valid {
                    bail!("插件资源声明无效");
                }
            }
            for index in 0..zip.len() {
                let mut entry = zip.by_index(index)?;
                if entry.is_dir() {
                    continue;
                }
                let path = target.join(entry.name().context("备份条目名无效")?.as_ref());
                std::fs::create_dir_all(path.parent().context("备份条目缺少父目录")?)?;
                let mut output = std::fs::File::create(&path)?;
                std::io::copy(&mut entry, &mut output)?;
                output.sync_all()?;
            }
            Ok(manifest)
        }
    })
    .await??;
    let mut connection = <sqlx::SqliteConnection as sqlx::Connection>::connect_with(
        &sqlx::sqlite::SqliteConnectOptions::new().filename(target.join("blog.sqlite")),
    )
    .await
    .context("恢复库无法打开")?;
    let integrity: String = sqlx::query_scalar("PRAGMA integrity_check")
        .fetch_one(&mut connection)
        .await
        .map_err(|error| anyhow::anyhow!("备份数据库完整性检查失败：{error}"))?;
    if integrity != "ok" {
        bail!("备份数据库完整性检查失败");
    }
    if let Some(paths) = checks["paths"].as_object() {
        for (key, relative) in paths {
            sqlx::query("INSERT INTO settings(key,value) VALUES(?,?) ON CONFLICT(key) DO UPDATE SET value=excluded.value")
                .bind(key)
                .bind(target.join(relative.as_str().unwrap_or("")).to_string_lossy().to_string())
                .execute(&mut connection)
                .await?;
        }
    }
    Ok(())
}

/// 执行定时备份任务：写入数据目录 backups/，按设置轮换，返回文件名与字节数。
pub async fn run(app: &App, ctx: &crate::tasks::Context) -> Result<Value> {
    let settings = crate::config::Settings::load(app).await?;
    let keep = settings.number("backup_keep").clamp(1, 30) as usize;
    let directory = app.data.join("backups");
    if tokio::fs::symlink_metadata(&directory)
        .await
        .map(|metadata| !metadata.is_dir())
        .unwrap_or(false)
    {
        bail!("自动备份目录不能是符号链接");
    }
    tokio::fs::create_dir_all(&directory).await?;
    let now = chrono::Utc::now().with_timezone(&app.config.timezone);
    let mut random = [0u8; 4];
    getrandom::fill(&mut random).context("生成随机数失败")?;
    let name = format!(
        "muke-auto-{}-{}.zip",
        now.format("%Y%m%d-%H%M%S"),
        hex::encode(random)
    );
    let archive = directory.join(&name);
    ctx.progress("创建备份", 0, None).await?;
    create(app, &archive, &ctx.stop).await?;
    // 只轮换本功能生成的文件，不清理手工备份
    let mut files: Vec<(String, std::time::SystemTime)> = Vec::new();
    let mut entries = tokio::fs::read_dir(&directory).await?;
    while let Some(entry) = entries.next_entry().await? {
        let file_name = entry.file_name().to_string_lossy().into_owned();
        if entry.file_type().await?.is_file()
            && name_pattern().is_match(&file_name)
            && file_name != name
        {
            let modified = entry
                .metadata()
                .await
                .and_then(|metadata| metadata.modified())
                .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
            files.push((file_name, modified));
        }
    }
    files.sort_by(|a, b| b.1.cmp(&a.1));
    for (file, _) in files.into_iter().skip(keep.saturating_sub(1)) {
        tokio::fs::remove_file(directory.join(file)).await?;
    }
    let bytes = tokio::fs::metadata(&archive).await?.len();
    Ok(json!({"ok":true,"file":name,"bytes":bytes}))
}

/// 列出自动备份文件（新→旧）。
pub async fn files(app: &App) -> Result<Vec<Value>> {
    let directory = app.data.join("backups");
    let mut files = Vec::new();
    let mut entries = match tokio::fs::read_dir(&directory).await {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(files),
        Err(error) => return Err(error.into()),
    };
    while let Some(entry) = entries.next_entry().await? {
        let file_name = entry.file_name().to_string_lossy().into_owned();
        if entry.file_type().await?.is_file() && name_pattern().is_match(&file_name) {
            let metadata = entry.metadata().await?;
            files.push(json!({
                "name": file_name,
                "bytes": metadata.len(),
                "created_at": metadata
                    .modified()
                    .ok()
                    .and_then(|time| time.duration_since(std::time::SystemTime::UNIX_EPOCH).ok())
                    .map(|duration| duration.as_secs())
                    .unwrap_or_default(),
            }));
        }
    }
    files.sort_by(|a, b| {
        b["created_at"]
            .as_i64()
            .unwrap_or_default()
            .cmp(&a["created_at"].as_i64().unwrap_or_default())
    });
    Ok(files)
}

fn name_pattern() -> &'static regex::Regex {
    static PATTERN: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(r"^muke-auto-\d{8}-\d{6}-[a-f0-9]{8}\.zip$").unwrap()
    });
    &PATTERN
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{config::Config, database, network, plugins, tasks};
    use std::sync::Arc;

    async fn app(root: &Path) -> App {
        let data = root.join("data");
        let config = Config {
            data_dir: data.clone(),
            export_dir: root.join("dist"),
            frontend: Path::new(env!("CARGO_MANIFEST_DIR")).join("frontend"),
            ..Config::default()
        };
        let app = App {
            db: database::open(&data).await.unwrap(),
            tasks: tasks::open(&root.join("work")).await.unwrap(),
            network: network::Client::new(None),
            data,
            config: Arc::new(config),
            shutdown: CancellationToken::new(),
            password_slots: Arc::new(tokio::sync::Semaphore::new(2)),
            media_slots: Arc::new(tokio::sync::Semaphore::new(2)),
            content_lock: Arc::new(tokio::sync::RwLock::new(())),
        };
        plugins::migrate(&app).await.unwrap();
        app
    }

    #[tokio::test]
    async fn backup_waits_for_content_mutations_and_can_cancel_while_waiting() {
        let root = tempfile::tempdir().unwrap();
        let app = app(root.path()).await;
        let archive = root.path().join("backup.zip");
        let guard = app.content_lock.write().await;
        let name = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa.png";
        std::fs::write(app.data.join("uploads").join(name), "old-image").unwrap();
        sqlx::query(
            "INSERT INTO posts(content,images,created_at,updated_at) VALUES('old',?,'2026','2026')",
        )
        .bind(json!([format!("up:{name}")]).to_string())
        .execute(&app.db)
        .await
        .unwrap();
        let stop = CancellationToken::new();
        let pending = create(&app, &archive, &stop);
        tokio::pin!(pending);
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(50), &mut pending)
                .await
                .is_err()
        );
        assert!(!archive.with_extension("sqlite.snapshot").exists());
        // Finish the mutation while backup is waiting, including its media deletion.
        sqlx::query("DELETE FROM posts")
            .execute(&app.db)
            .await
            .unwrap();
        std::fs::remove_file(app.data.join("uploads").join(name)).unwrap();
        stop.cancel();
        assert!(
            tokio::time::timeout(std::time::Duration::from_secs(1), &mut pending)
                .await
                .unwrap()
                .is_err()
        );
        assert!(!archive.exists());
        drop(guard);
        create(&app, &archive, &CancellationToken::new())
            .await
            .unwrap();
        let restored = root.path().join("restored");
        std::fs::create_dir(&restored).unwrap();
        restore(&archive, &restored).await.unwrap();
        let db = database::open(&restored).await.unwrap();
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM posts")
                .fetch_one(&db)
                .await
                .unwrap(),
            0
        );
        assert!(!restored.join("uploads").join(name).exists());
        assert!(app.content_lock.try_write().is_ok());
    }

    #[tokio::test]
    async fn backup_round_trips_database_uploads_and_plugin_paths() {
        let root = tempfile::tempdir().unwrap();
        let app = app(root.path()).await;
        let name = format!("{}.png", "a".repeat(32));
        image::RgbImage::new(40, 30)
            .save(app.data.join("uploads").join(&name))
            .unwrap();
        image::RgbImage::new(20, 20)
            .save(app.data.join("avatar.webp"))
            .unwrap();
        sqlx::query("INSERT INTO posts(content,source,images,meta,created_at,updated_at) VALUES('内容','','[]','{}','2026-11-26','2026-11-26')")
            .execute(&app.db)
            .await
            .unwrap();
        let wallpapers = root.path().join("pictures");
        std::fs::create_dir_all(wallpapers.join("sub")).unwrap();
        std::fs::write(wallpapers.join("wall.png"), b"wallpaper").unwrap();
        std::fs::write(wallpapers.join("sub").join("deep.txt"), b"deep").unwrap();
        for (key, value) in [
            ("wallpapers_enabled", "1"),
            ("wallpaper_dir", wallpapers.to_str().unwrap()),
        ] {
            sqlx::query("INSERT INTO settings VALUES(?,?)")
                .bind(key)
                .bind(value)
                .execute(&app.db)
                .await
                .unwrap();
        }
        let archive = root.path().join("backup.zip");
        create(&app, &archive, &CancellationToken::new())
            .await
            .unwrap();
        assert!(archive.is_file());
        let file = std::fs::File::open(&archive).unwrap();
        let mut zip = zip::ZipArchive::new(file).unwrap();
        let mut names: Vec<String> = (0..zip.len())
            .map(|index| zip.by_index(index).unwrap().name().unwrap().to_string())
            .collect();
        names.sort();
        assert!(names.contains(&"blog.sqlite".to_owned()));
        assert!(names.contains(&"manifest.json".to_owned()));
        assert!(names.contains(&format!("uploads/{name}")));
        assert!(names.contains(&"avatar.webp".to_owned()));
        assert!(names.contains(&"plugins/wallpapers/wallpaper_dir/wall.png".to_owned()));
        assert!(names.contains(&"plugins/wallpapers/wallpaper_dir/sub/deep.txt".to_owned()));
        // 已存在目标拒绝；恢复到空目录
        assert!(
            create(&app, &archive, &CancellationToken::new())
                .await
                .unwrap_err()
                .to_string()
                .contains("备份目标已存在")
        );
        let target = root.path().join("restored");
        std::fs::create_dir(&target).unwrap();
        restore(&archive, &target).await.unwrap();
        let restored = target.join("blog.sqlite");
        let mut connection = <sqlx::SqliteConnection as sqlx::Connection>::connect_with(
            &sqlx::sqlite::SqliteConnectOptions::new().filename(&restored),
        )
        .await
        .unwrap();
        let posts: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM posts")
            .fetch_one(&mut connection)
            .await
            .unwrap();
        assert_eq!(posts, 1);
        let directory: String =
            sqlx::query_scalar("SELECT value FROM settings WHERE key='wallpaper_dir'")
                .fetch_one(&mut connection)
                .await
                .unwrap();
        assert_eq!(
            Path::new(&directory),
            target.join("plugins/wallpapers/wallpaper_dir")
        );
        assert_eq!(
            std::fs::read(target.join("uploads").join(&name)).unwrap(),
            std::fs::read(app.data.join("uploads").join(&name)).unwrap()
        );
        assert_eq!(
            std::fs::read(target.join("plugins/wallpapers/wallpaper_dir/wall.png")).unwrap(),
            b"wallpaper"
        );
        // 非空目录拒绝、坏清单拒绝、取消
        assert!(
            restore(&archive, &target)
                .await
                .unwrap_err()
                .to_string()
                .contains("空目录")
        );
        let bad = root.path().join("bad.zip");
        {
            let file = std::fs::File::create(&bad).unwrap();
            let mut writer = zip::ZipWriter::new(file);
            let options = SimpleFileOptions::default();
            writer.start_file("manifest.json", options).unwrap();
            writer
                .write_all(br#"{"app":"other","version":1,"paths":{}}"#)
                .unwrap();
            writer.finish().unwrap();
        }
        let empty = root.path().join("empty");
        std::fs::create_dir(&empty).unwrap();
        assert!(
            restore(&bad, &empty)
                .await
                .unwrap_err()
                .to_string()
                .contains("备份格式不正确")
        );
        let cancelled = CancellationToken::new();
        cancelled.cancel();
        assert!(
            create(&app, &root.path().join("cancelled.zip"), &cancelled)
                .await
                .is_err()
        );
        assert!(!root.path().join("cancelled.zip").exists());
        assert!(!root.path().join("cancelled.sqlite.snapshot").exists());
    }

    #[tokio::test]
    async fn scheduled_task_creates_and_rotates_backups() {
        let root = tempfile::tempdir().unwrap();
        let app = app(root.path()).await;
        for (key, value) in [
            ("backup_enabled", "1"),
            ("backup_interval_hours", "1"),
            ("backup_keep", "2"),
        ] {
            sqlx::query("INSERT INTO settings VALUES(?,?)")
                .bind(key)
                .bind(value)
                .execute(&app.db)
                .await
                .unwrap();
        }
        sqlx::query("INSERT OR IGNORE INTO jobs(id,kind,resource,dedup,payload,state) VALUES(1,'backup','backup','fixture','{}','running')")
            .execute(&app.tasks)
            .await
            .unwrap();
        let ctx = tasks::Context {
            id: 1,
            pool: app.tasks.clone(),
            directory: root.path().join("job-1"),
            stop: CancellationToken::new(),
        };
        let first = run(&app, &ctx).await.unwrap();
        assert!(name_pattern().is_match(first["file"].as_str().unwrap()));
        std::thread::sleep(std::time::Duration::from_millis(1100));
        let second = run(&app, &ctx).await.unwrap();
        std::thread::sleep(std::time::Duration::from_millis(1100));
        let third = run(&app, &ctx).await.unwrap();
        let listed = files(&app).await.unwrap();
        assert_eq!(listed.len(), 2);
        assert_eq!(
            listed[0]["name"].as_str().unwrap(),
            third["file"].as_str().unwrap()
        );
        assert_eq!(
            listed[1]["name"].as_str().unwrap(),
            second["file"].as_str().unwrap()
        );
        assert_ne!(
            listed[1]["name"].as_str().unwrap(),
            first["file"].as_str().unwrap()
        );
        let schedule = tasks::submit(&app.tasks, &tasks::Task::Backup {})
            .await
            .unwrap();
        assert!(schedule > 0);
    }
}
