use super::snapshot::Snapshot;
use crate::{
    api::{App, upload_name},
    content, media, plugins,
};
use anyhow::{Result, bail};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};
use tokio_util::sync::CancellationToken;

pub struct Copies {
    sources: BTreeMap<String, PathBuf>,
    thumbnails: BTreeSet<String>,
    names: BTreeMap<String, String>,
    size: i64,
    quality: i64,
}

pub async fn copy_sources(
    app: &App,
    snapshot: &mut Snapshot<'_>,
    destination: &Path,
    stop: &CancellationToken,
) -> Result<Copies> {
    if stop.is_cancelled() {
        bail!("任务已取消");
    }
    let mut references = BTreeSet::new();
    for post in &snapshot.posts {
        let images: Vec<String> =
            serde_json::from_str(post["images"].as_str().unwrap_or("[]")).unwrap_or_default();
        references.extend(images);
        references.extend(plugins::post_refs(
            &app.plugins,
            post["source"].as_str().unwrap_or(""),
            post["meta"].as_str().unwrap_or(""),
        ));
    }
    for plugin in snapshot.plugins.values() {
        references.extend(plugin.references.iter().cloned());
    }
    for article in &snapshot.articles {
        references.extend(
            content::image_refs(article["content"].as_str().unwrap_or(""))
                .into_iter()
                .map(|name| format!("up:{name}")),
        );
    }
    for directory in [
        "media/uploads",
        "media/plugins",
        "thumbnails",
        ".media-sources",
    ] {
        tokio::fs::create_dir_all(destination.join(directory)).await?;
    }
    let mut copies = Copies {
        sources: BTreeMap::new(),
        thumbnails: BTreeSet::new(),
        names: BTreeMap::new(),
        size: snapshot.state["options"]["thumb_size"].as_i64().unwrap(),
        quality: snapshot.state["options"]["thumb_quality"].as_i64().unwrap(),
    };
    for reference in references {
        if stop.is_cancelled() {
            bail!("任务已取消");
        }
        let source = if let Some(name) = reference.strip_prefix("up:") {
            if !upload_name(name) {
                continue;
            }
            let root = app.data.join("uploads");
            if !tokio::fs::symlink_metadata(&root).await?.is_dir() {
                bail!("上传目录不能是符号链接");
            }
            let source = root.join(name);
            match tokio::fs::symlink_metadata(&source).await {
                Ok(_) => Some(source),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
                Err(error) => return Err(error.into()),
            }
        } else {
            plugins::export_source(app, &reference).await?
        };
        let Some(source) = source else {
            continue;
        };
        if !reference.starts_with("up:")
            && let Some(value) = plugins::resolve_ref(app, &reference).await?
            && let Some(name) = value["name"].as_str()
        {
            copies.names.insert(reference.clone(), name.to_owned());
        }
        let key = crate::auth::digest(&reference);
        let target = if let Some(name) = reference.strip_prefix("up:") {
            destination.join("media/uploads").join(name)
        } else {
            destination.join(".media-sources").join(&key)
        };
        copy_file(&source, &target, stop).await?;
        if let Some(thumbnail) = plugins::thumbnail(app, &reference).await {
            copy_file(
                &thumbnail,
                &destination.join("thumbnails").join(format!("{key}.webp")),
                stop,
            )
            .await?;
            copies.thumbnails.insert(reference.clone());
        }
        copies.sources.insert(reference, target);
    }
    for kind in ["avatar", "cover"] {
        if !snapshot.state["profile"][kind].is_null() {
            let relative = format!("media/{kind}.webp");
            copy_file(
                &app.data.join(format!("{kind}.webp")),
                &destination.join(&relative),
                stop,
            )
            .await?;
            snapshot.state["profile"][kind] = json!(format!("/{relative}"));
        }
    }
    Ok(copies)
}

impl Copies {
    pub async fn finish(
        self,
        app: &App,
        destination: &Path,
        stop: &CancellationToken,
    ) -> Result<BTreeMap<String, Value>> {
        if stop.is_cancelled() {
            bail!("任务已取消");
        }
        let mut descriptors = BTreeMap::new();
        for (reference, mut source) in self.sources {
            let (width, height, format) = media::inspect(app, &source, stop).await?;
            let key = crate::auth::digest(&reference);
            let relative = if let Some(name) = reference.strip_prefix("up:") {
                format!("media/uploads/{name}")
            } else {
                let key = fingerprint(&source, stop).await?;
                let relative = format!("media/plugins/{key}.{format}");
                let target = destination.join(&relative);
                tokio::fs::rename(&source, &target).await?;
                source = target;
                relative
            };
            let thumbnail = format!("thumbnails/{key}.webp");
            if !self.thumbnails.contains(&reference) {
                let _permit = tokio::select! { _ = stop.cancelled() => bail!("任务已取消"), permit = app.media_slots.acquire() => permit? };
                media::thumbnail(
                    &source,
                    &destination.join(&thumbnail),
                    self.size,
                    self.quality,
                    stop,
                )
                .await?;
            }
            let thumb_path = destination.join(&thumbnail);
            let thumbnail = format!("thumbnails/{}.webp", fingerprint(&thumb_path, stop).await?);
            tokio::fs::rename(&thumb_path, destination.join(&thumbnail)).await?;
            let bytes = tokio::fs::metadata(&source).await?.len();
            descriptors.insert(reference.clone(), json!({"ref":reference,"url":format!("/{relative}"),"thumb":format!("/{thumbnail}"),"width":width,"height":height,"bytes":bytes}));
        }
        for (reference, name) in self.names {
            if let Some(descriptor) = descriptors.get_mut(&reference) {
                descriptor["name"] = json!(name);
            }
        }
        tokio::fs::remove_dir(destination.join(".media-sources")).await?;
        Ok(descriptors)
    }
}

async fn fingerprint(path: &Path, stop: &CancellationToken) -> Result<String> {
    use sha2::{Digest, Sha256};
    use tokio::io::AsyncReadExt;
    let mut file = tokio::fs::File::open(path).await?;
    let mut hash = Sha256::new();
    let mut buffer = vec![0; 64 * 1024];
    loop {
        if stop.is_cancelled() {
            bail!("任务已取消");
        }
        let size = file.read(&mut buffer).await?;
        if size == 0 {
            return Ok(hex::encode(hash.finalize()));
        }
        hash.update(&buffer[..size]);
    }
}

async fn copy_file(source: &Path, target: &Path, stop: &CancellationToken) -> Result<()> {
    let mut input = tokio::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(source)
        .await?;
    let before = input.metadata().await?;
    if !before.is_file() {
        bail!("导出媒体必须是普通文件");
    }
    let mut output = tokio::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(target)
        .await?;
    let bytes = tokio::select! { _ = stop.cancelled() => bail!("任务已取消"), bytes = tokio::io::copy(&mut input, &mut output) => bytes? };
    use tokio::io::AsyncWriteExt;
    output.flush().await?;
    let after = input.metadata().await?;
    if bytes != before.len()
        || after.len() != before.len()
        || after.modified()? != before.modified()?
    {
        bail!("导出时媒体已变化，请重试");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        config::Config,
        database,
        export::{artifact::Artifact, snapshot},
        network, tasks,
    };
    use std::sync::Arc;

    #[tokio::test]
    async fn copies_only_public_references_and_processes_after_unlock() {
        let root = tempfile::tempdir().unwrap();
        let data = root.path().join("data");
        let app = App {
            plugins: crate::plugins::Registry::default(),
            db: database::open(&data).await.unwrap(),
            tasks: tasks::open(&root.path().join("work")).await.unwrap(),
            network: network::Client::new(None),
            data,
            config: Arc::new(Config::default()),
            shutdown: CancellationToken::new(),
            password_slots: Arc::new(tokio::sync::Semaphore::new(2)),
            media_slots: Arc::new(tokio::sync::Semaphore::new(2)),
            content_lock: Arc::new(tokio::sync::RwLock::new(())),
        };
        plugins::migrate(&app).await.unwrap();
        let name = format!("{}.png", "a".repeat(32));
        let draft = format!("{}.png", "b".repeat(32));
        let image = image::RgbImage::new(160, 120);
        image.save(app.data.join("uploads").join(&name)).unwrap();
        image.save(app.data.join("uploads").join(&draft)).unwrap();
        image.save(app.data.join("avatar.webp")).unwrap();
        let wallpaper = root.path().join("wallpapers");
        std::fs::create_dir(&wallpaper).unwrap();
        image.save(wallpaper.join("picture.png")).unwrap();
        image.save(wallpaper.join("gallery-only.png")).unwrap();
        for (key, value) in [
            ("wallpapers_enabled", "1"),
            ("plugin:rss:enabled", "1"),
            ("gallery_batch", "12"),
        ] {
            sqlx::query("INSERT INTO settings VALUES(?,?)")
                .bind(key)
                .bind(value)
                .execute(&app.db)
                .await
                .unwrap();
        }
        sqlx::query("INSERT INTO rss_feeds(id,url,title,created_at,last_error) VALUES('0123456789ab','https://internal.example/secret-token','Public title','2026','secret-error')").execute(&app.db).await.unwrap();
        sqlx::query("INSERT INTO wallpapers(id,path,name,width,height,bytes,mtime) VALUES('gallery','gallery-only.png','Gallery only',1,1,1,1)").execute(&app.db).await.unwrap();
        std::fs::rename(wallpaper.join("picture.png"), wallpaper.join("picture.jpg")).unwrap();
        sqlx::query("INSERT INTO settings VALUES('wallpaper_dir',?)")
            .bind(wallpaper.to_str().unwrap())
            .execute(&app.db)
            .await
            .unwrap();
        sqlx::query("INSERT INTO wallpapers(id,path,name,width,height,bytes,mtime) VALUES('test','picture.jpg','picture',160,120,0,0)").execute(&app.db).await.unwrap();
        sqlx::query("INSERT INTO posts(content,images,meta,source,created_at,updated_at) VALUES('post',?,?,'rss','2026','2026')")
            .bind(json!([format!("up:{name}"), "wall:test", "up:../../private"]).to_string())
            .bind(json!({"avatar":format!("up:{name}")}).to_string()).execute(&app.db).await.unwrap();
        sqlx::query("UPDATE posts SET source_key='rss:0123456789ab:one'")
            .execute(&app.db)
            .await
            .unwrap();
        for (file, draft) in [(&name, 0), (&draft, 1)] {
            sqlx::query("INSERT INTO articles(title,content,draft,created_at,updated_at) VALUES('article',?,?,'2026','2026')")
                .bind(format!("![](/image?upload={file})")).bind(draft).execute(&app.db).await.unwrap();
        }
        std::fs::create_dir_all(app.data.join("rss-thumbs")).unwrap();
        let thumbnail = app.data.join("rss-thumbs").join(format!("{name}.webp"));
        image::RgbImage::new(32, 24).save(&thumbnail).unwrap();
        let cached_thumbnail = std::fs::read(thumbnail).unwrap();
        let stop = CancellationToken::new();
        let artifact = Artifact::new(&root.path().join("export")).unwrap();
        let mut snapshot = snapshot::capture(&app, &stop).await.unwrap();
        let copies = copy_sources(&app, &mut snapshot, artifact.path(), &stop)
            .await
            .unwrap();
        assert_eq!(snapshot.state["profile"]["avatar"], "/media/avatar.webp");
        assert!(app.content_lock.try_write().is_err());
        let plugin_snapshots = std::mem::take(&mut snapshot.plugins);
        assert_eq!(snapshot.hidden_sources, vec!["rss"]);
        drop(snapshot);
        let writer = app.content_lock.try_write().unwrap();
        std::fs::remove_file(app.data.join("uploads").join(&name)).unwrap();
        std::fs::remove_file(wallpaper.join("picture.jpg")).unwrap();
        drop(writer);
        let descriptors = copies.finish(&app, artifact.path(), &stop).await.unwrap();
        assert_eq!(descriptors.len(), 3);
        assert_eq!(descriptors["wall:test"]["name"], "picture");
        let posts = vec![
            json!({"source":"rss","id":1,"images":[descriptors[&format!("up:{name}")]]}),
            json!({"source":"","id":2}),
        ];
        let (items, metadata) =
            plugins::export_data(&app.plugins, &plugin_snapshots, &posts, &descriptors);
        assert_eq!(items["rss"]["posts"].as_array().unwrap().len(), 1);
        assert_eq!(items["rss"]["posts"][0], posts[0]);
        assert_eq!(items["rss"]["feeds"][0]["count"], 1);
        assert_eq!(metadata["rss"]["total"], 1);
        assert_eq!(items["wallpapers"].as_array().unwrap().len(), 2);
        assert_eq!(metadata["wallpapers"]["batch"], 12);
        let gallery = items["wallpapers"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["id"] == "gallery")
            .unwrap();
        assert_eq!(gallery["width"], 160);
        assert_eq!(gallery["original"], descriptors["wall:gallery"]["url"]);
        let serialized = items.to_string();
        assert!(!serialized.contains("secret-token") && !serialized.contains("secret-error"));
        assert!(!serialized.contains("\"path\"") && !serialized.contains("/image?"));
        assert_eq!(descriptors[&format!("up:{name}")]["width"], 160);
        assert!(
            descriptors["wall:test"]["url"]
                .as_str()
                .unwrap()
                .ends_with(".png")
        );
        let thumb = descriptors[&format!("up:{name}")]["thumb"]
            .as_str()
            .unwrap();
        assert_eq!(
            std::fs::read(artifact.path().join(&thumb[1..])).unwrap(),
            cached_thumbnail
        );
        assert!(!artifact.path().join("media/uploads").join(draft).exists());
        assert!(!artifact.path().join(".media-sources").exists());
        assert!(artifact.path().join("media/avatar.webp").exists());
        let link = root.path().join("unsafe.png");
        std::os::unix::fs::symlink(app.data.join("avatar.webp"), &link).unwrap();
        assert!(
            copy_file(&link, &root.path().join("leak"), &stop)
                .await
                .is_err()
        );
        assert!(!root.path().join("leak").exists());
        for key in ["wallpapers_enabled", "plugin:rss:enabled"] {
            sqlx::query("UPDATE settings SET value='0' WHERE key=?")
                .bind(key)
                .execute(&app.db)
                .await
                .unwrap();
        }
        assert!(plugins::export_snapshots(&app).await.unwrap().is_empty());
        let cancelled = CancellationToken::new();
        cancelled.cancel();
        let mut snapshot = snapshot::capture(&app, &stop).await.unwrap();
        assert!(
            copy_sources(&app, &mut snapshot, artifact.path(), &cancelled)
                .await
                .is_err()
        );
    }
}
