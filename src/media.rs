use crate::{
    api::App,
    auth,
    config::Settings,
    error::{Error, Result},
};
use anyhow::{Context, bail};
use std::{path::Path, process::Stdio, time::Duration};
use tokio::process::Command;
use tokio_util::sync::CancellationToken;

fn command() -> Command {
    let mut command = Command::new("magick");
    let parent = std::process::id() as libc::pid_t;
    unsafe {
        command.pre_exec(move || {
            if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL) == -1 {
                return Err(std::io::Error::last_os_error());
            }
            if libc::getppid() != parent {
                libc::_exit(1);
            }
            Ok(())
        });
    }
    command.args([
        "-limit", "thread", "1", "-limit", "memory", "128MiB", "-limit", "map", "256MiB",
    ]);
    command
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    command
}
pub async fn inspect(
    app: &App,
    path: &Path,
    stop: &CancellationToken,
) -> Result<(u32, u32, String)> {
    use tokio::io::AsyncReadExt;
    let mut header = [0u8; 32];
    let mut file = tokio::fs::File::open(path).await?;
    let count = file.read(&mut header).await?;
    if !matches!(
        image::guess_format(&header[..count]),
        Ok(image::ImageFormat::Jpeg
            | image::ImageFormat::Png
            | image::ImageFormat::Gif
            | image::ImageFormat::WebP
            | image::ImageFormat::Avif)
    ) {
        return Err(Error::invalid("图片格式无效"));
    }
    let _permit = tokio::select! {_=stop.cancelled()=>return Err(Error::invalid("任务已取消")), permit=app.media_slots.acquire()=>permit.map_err(|e|Error::Internal(e.to_string()))?};
    let mut child = command()
        .arg(format!("{}[0]", path.display()))
        .args(["-ping", "-format", "%w %h %m", "info:"])
        .stdout(Stdio::piped())
        .spawn()?;
    let stdout = child.stdout.take().expect("piped stdout");
    let reader = tokio::spawn(async move {
        use tokio::io::AsyncReadExt;
        let mut bytes = Vec::new();
        stdout.take(1024).read_to_end(&mut bytes).await?;
        Ok::<_, std::io::Error>(bytes)
    });
    let status = tokio::select! {
        status=tokio::time::timeout(Duration::from_secs(15),child.wait())=>status,
        _=stop.cancelled()=>{child.kill().await?;child.wait().await?;reader.abort();return Err(Error::invalid("任务已取消"));}
    };
    let status = match status {
        Ok(status) => status?,
        Err(_) => {
            child.kill().await?;
            child.wait().await?;
            reader.abort();
            return Err(Error::invalid("图片检测超时"));
        }
    };
    let bytes = reader.await.map_err(|e| Error::Internal(e.to_string()))??;
    if !status.success() {
        return Err(Error::invalid("无法读取图片"));
    }
    let text = String::from_utf8_lossy(&bytes);
    let fields: Vec<_> = text.split_whitespace().collect();
    if fields.len() != 3 {
        return Err(Error::invalid("图片格式无效"));
    }
    let width = fields[0]
        .parse()
        .map_err(|_| Error::invalid("图片尺寸无效"))?;
    let height = fields[1]
        .parse()
        .map_err(|_| Error::invalid("图片尺寸无效"))?;
    let ext = match fields[2] {
        "JPEG" => "jpg",
        "PNG" => "png",
        "GIF" => "gif",
        "WEBP" => "webp",
        "AVIF" | "HEIC" => "avif",
        _ => return Err(Error::invalid("仅支持 JPEG / PNG / WebP / GIF / AVIF 图片")),
    };
    Ok((width, height, ext.into()))
}
pub async fn store(app: &App, bytes: &[u8]) -> Result<String> {
    let max = Settings::load(app).await?.number("upload_max_mb") as usize * 1024 * 1024;
    if bytes.len() > max {
        return Err(Error::invalid(format!(
            "单张图片不能超过 {}MB",
            max / 1024 / 1024
        )));
    }
    let name = &auth::random_token()?[..32];
    let temp = app.data.join("uploads").join(format!(".upload-{name}"));
    tokio::fs::write(&temp, bytes).await?;
    let result = async {
        let (_, _, ext) = inspect(app, &temp, &app.shutdown).await?;
        let filename = format!("{name}.{ext}");
        tokio::fs::rename(&temp, app.data.join("uploads").join(&filename)).await?;
        Ok(format!("up:{filename}"))
    }
    .await;
    if result.is_err() {
        let _ = tokio::fs::remove_file(&temp).await;
    }
    result
}
pub async fn delete_unused(app: &App, reference: &str) -> Result<()> {
    let Some(name) = reference
        .strip_prefix("up:")
        .filter(|s| crate::api::upload_name(s))
    else {
        return Ok(());
    };
    let used:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM articles WHERE images LIKE ? UNION ALL SELECT 1 FROM article_revisions WHERE images LIKE ? UNION ALL SELECT 1 FROM posts WHERE images LIKE ?)")
        .bind(format!("%\"{name}\"%")).bind(format!("%\"{name}\"%")).bind(format!("%\"up:{name}\"%")).fetch_one(&app.db).await?;
    if !used && !crate::plugins::references(app, reference).await? {
        remove_file(&app.data.join("uploads").join(name)).await?;
        crate::plugins::delete_media(app, reference).await?;
        remove_file(&app.data.join("thumbs").join(format!("{name}.webp"))).await?;
        match tokio::fs::remove_dir_all(app.data.join("thumbs").join(name)).await {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}
pub async fn remove_file(path: &Path) -> Result<()> {
    match tokio::fs::remove_file(path).await {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.into()),
    }
}

pub async fn compress(
    source: &str,
    target: &Path,
    limit: u64,
    initial_size: u32,
    stop: &CancellationToken,
) -> anyhow::Result<()> {
    for size in [
        initial_size,
        initial_size * 3 / 4,
        initial_size / 2,
        initial_size / 4,
    ] {
        for quality in [80, 60, 40, 20] {
            if stop.is_cancelled() {
                bail!("任务已取消");
            }
            let mut child = command()
                .arg(format!("{source}[0]"))
                .args(["-auto-orient", "-thumbnail"])
                .arg(format!("{size}x{size}>"))
                .args(["-strip", "-quality", &quality.to_string()])
                .arg(target)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .kill_on_drop(true)
                .spawn()
                .context("无法启动 ImageMagick，请检查 magick 命令")?;
            let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
            loop {
                tokio::select! {
                    status=child.wait()=>{ if !status?.success() { bail!("ImageMagick 图片处理失败"); } break; },
                    _=tokio::time::sleep_until(deadline)=>{ child.kill().await?; child.wait().await?; bail!("图片处理超时"); },
                    _=tokio::time::sleep(Duration::from_millis(100))=>{
                        if stop.is_cancelled() { child.kill().await?; child.wait().await?; bail!("任务已取消"); }
                    }
                }
            }
            let bytes = tokio::fs::metadata(target).await?.len();
            if bytes > 0 && bytes < limit {
                return Ok(());
            }
        }
    }
    bail!("无法将图片压缩到大小限制以下")
}

pub async fn thumbnail(
    source: &Path,
    target: &Path,
    size: i64,
    quality: i64,
    stop: &CancellationToken,
) -> anyhow::Result<()> {
    let mut child = command()
        .arg(format!("{}[0]", source.display()))
        .args([
            "-auto-orient",
            "-thumbnail",
            &format!("{size}x{size}>"),
            "-strip",
            "-quality",
            &quality.to_string(),
        ])
        .arg(target)
        .stdout(Stdio::null())
        .spawn()?;
    tokio::select! {
        status=child.wait()=>{if !status?.success(){bail!("缩略图生成失败");}},
        _=stop.cancelled()=>{child.kill().await?;child.wait().await?;bail!("任务已取消");},
        _=tokio::time::sleep(Duration::from_secs(30))=>{child.kill().await?;child.wait().await?;bail!("缩略图生成超时");}
    }
    Ok(())
}

pub struct Upload {
    pub field: String,
    pub name: String,
    pub bytes: Vec<u8>,
}

#[derive(Clone, Copy)]
pub enum ProfileImage {
    Avatar,
    Cover,
}
impl ProfileImage {
    fn name(self) -> &'static str {
        match self {
            Self::Avatar => "avatar",
            Self::Cover => "cover",
        }
    }
}

pub async fn recover(app: &App) -> Result<()> {
    let root = app.data.join(".media-staging");
    tokio::fs::create_dir_all(&root).await?;
    if !tokio::fs::symlink_metadata(&root).await?.is_dir() {
        return Err(Error::invalid("媒体暂存目录不能为符号链接"));
    }
    let mut entries = tokio::fs::read_dir(root).await?;
    while let Some(entry) = entries.next_entry().await? {
        if ["profile-", "thumbnail-", "gravatar-"]
            .iter()
            .any(|prefix| entry.file_name().to_string_lossy().starts_with(prefix))
            && entry.file_type().await?.is_dir()
        {
            tokio::fs::remove_dir_all(entry.path()).await?;
        }
    }
    Ok(())
}

pub async fn profile_url(app: &App, kind: ProfileImage) -> Result<Option<String>> {
    let metadata =
        match tokio::fs::symlink_metadata(app.data.join(format!("{}.webp", kind.name()))).await {
            Ok(metadata) if metadata.is_file() => metadata,
            Ok(_) => return Ok(None),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
        };
    let version = metadata
        .modified()?
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    Ok(Some(format!("/image?{}=1&v={version}", kind.name())))
}

pub async fn profile_response(app: &App, kind: ProfileImage) -> Result<axum::response::Response> {
    use axum::response::IntoResponse;
    if profile_url(app, kind).await?.is_none() {
        return Err(Error::not_found("图片不存在"));
    }
    let mut service = tower_http::services::ServeFile::new_with_mime(
        app.data.join(format!("{}.webp", kind.name())),
        &"image/webp".parse().unwrap(),
    );
    let response = service
        .try_call(axum::http::Request::new(axum::body::Body::empty()))
        .await?;
    Ok(response.into_response())
}

pub async fn save_profile(
    app: &App,
    kind: ProfileImage,
    bytes: &[u8],
) -> Result<serde_json::Value> {
    let settings = Settings::load(app).await?;
    if bytes.len() > settings.number("upload_max_mb") as usize * 1024 * 1024 {
        return Err(Error::invalid("图片超过上传大小限制"));
    }
    let staging = app.data.join(".media-staging");
    tokio::fs::create_dir_all(&staging).await?;
    if !tokio::fs::symlink_metadata(&staging).await?.is_dir() {
        return Err(Error::invalid("媒体暂存目录不能为符号链接"));
    }
    let temporary = tempfile::Builder::new()
        .prefix("profile-")
        .tempdir_in(staging)?;
    let source = temporary.path().join("source");
    let target = temporary.path().join("image.webp");
    tokio::fs::write(&source, bytes).await?;
    inspect(app, &source, &app.shutdown).await?;
    let size = match kind {
        ProfileImage::Avatar => format!("{0}x{0}", settings.number("avatar_size")),
        ProfileImage::Cover => "1500x500".into(),
    };
    {
        let _permit = tokio::select! {_=app.shutdown.cancelled()=>return Err(Error::invalid("服务正在关闭")),permit=app.media_slots.acquire()=>permit.map_err(|e|Error::Internal(e.to_string()))?};
        let mut child = command()
            .arg(format!("{}[0]", source.display()))
            .args([
                "-auto-orient",
                "-thumbnail",
                &format!("{size}^"),
                "-gravity",
                "center",
                "-extent",
                &size,
                "-strip",
                "-quality",
                "85",
            ])
            .arg(&target)
            .stdout(Stdio::null())
            .spawn()?;
        tokio::select! {
            status=child.wait()=>{if !status?.success(){return Err(Error::invalid("图片处理失败"));}},
            _=app.shutdown.cancelled()=>{child.kill().await?;child.wait().await?;return Err(Error::invalid("服务正在关闭"));},
            _=tokio::time::sleep(Duration::from_secs(30))=>{child.kill().await?;child.wait().await?;return Err(Error::invalid("图片处理超时"));},
        }
    }
    if tokio::fs::metadata(&target).await?.len() == 0 {
        return Err(Error::invalid("图片处理失败"));
    }
    let _guard = app.content_lock.write().await;
    tokio::fs::rename(target, app.data.join(format!("{}.webp", kind.name()))).await?;
    Ok(serde_json::json!({"ok":true,kind.name():profile_url(app,kind).await?}))
}

pub async fn remove_profile(app: &App, kind: ProfileImage) -> Result<serde_json::Value> {
    let _guard = app.content_lock.write().await;
    remove_file(&app.data.join(format!("{}.webp", kind.name()))).await?;
    Ok(serde_json::json!({"ok":true}))
}

// Fixed lock stripes bound memory while serializing requests for the same image.
static MEDIA_LOCKS: std::sync::LazyLock<[tokio::sync::Mutex<()>; 64]> =
    std::sync::LazyLock::new(|| std::array::from_fn(|_| tokio::sync::Mutex::new(())));

pub async fn cached_thumbnail(
    app: &App,
    name: &str,
    source: &Path,
) -> anyhow::Result<std::path::PathBuf> {
    let index = u8::from_str_radix(&name[..2], 16)? as usize % MEDIA_LOCKS.len();
    let _guard = tokio::select! {_=app.shutdown.cancelled()=>bail!("服务正在关闭"),guard=MEDIA_LOCKS[index].lock()=>guard};
    let settings = Settings::load(app).await?;
    let before = tokio::fs::metadata(source).await?;
    let modified = before.modified()?;
    let version = auth::digest(&format!(
        "{}|{}|{}|{}",
        modified.duration_since(std::time::UNIX_EPOCH)?.as_nanos(),
        before.len(),
        settings.number("thumb_size"),
        settings.number("thumb_quality")
    ));
    let cache = app.data.join("thumbs").join(name);
    let target = cache.join(format!("{version}.webp"));
    if tokio::fs::symlink_metadata(&target)
        .await
        .is_ok_and(|m| m.is_file())
        && tokio::fs::symlink_metadata(&cache)
            .await
            .is_ok_and(|m| m.is_dir())
        && tokio::fs::symlink_metadata(app.data.join("thumbs"))
            .await
            .is_ok_and(|m| m.is_dir())
    {
        return Ok(target);
    }
    let staging = app.data.join(".media-staging");
    if !tokio::fs::symlink_metadata(&staging).await?.is_dir() {
        bail!("媒体暂存目录无效");
    }
    let temporary = tempfile::Builder::new()
        .prefix("thumbnail-")
        .tempdir_in(staging)?;
    let output = temporary.path().join("image.webp");
    inspect(app, source, &app.shutdown).await?;
    {
        let _permit = tokio::select! {_=app.shutdown.cancelled()=>bail!("服务正在关闭"),permit=app.media_slots.acquire()=>permit?};
        thumbnail(
            source,
            &output,
            settings.number("thumb_size"),
            settings.number("thumb_quality"),
            &app.shutdown,
        )
        .await?;
    }
    let _content = app.content_lock.write().await;
    let after = tokio::fs::metadata(source).await?;
    if after.modified()? != modified || after.len() != before.len() {
        bail!("原图已改变，请重试");
    }
    let cache_root = app.data.join("thumbs");
    tokio::fs::create_dir_all(&cache_root).await?;
    if !tokio::fs::symlink_metadata(&cache_root).await?.is_dir() {
        bail!("缩略图目录无效");
    }
    tokio::fs::create_dir_all(&cache).await?;
    if !tokio::fs::symlink_metadata(&cache).await?.is_dir() {
        bail!("缩略图目录无效");
    }
    tokio::fs::rename(output, &target).await?;
    let mut entries = tokio::fs::read_dir(&cache).await?;
    while let Some(entry) = entries.next_entry().await? {
        if entry.path() != target {
            remove_file(&entry.path()).await?;
        }
    }
    Ok(target)
}

pub async fn gravatar(app: &App, hash: &str) -> Result<axum::response::Response> {
    use axum::response::IntoResponse;
    use tokio::io::AsyncReadExt;
    let missing = || Error::not_found("头像不存在");
    if hash.len() != 32
        || !hash
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
    {
        return Err(missing());
    }
    let index = u8::from_str_radix(&hash[..2], 16).unwrap() as usize % MEDIA_LOCKS.len();
    let _guard = tokio::select! {_=app.shutdown.cancelled()=>return Err(missing()),guard=MEDIA_LOCKS[index].lock()=>guard};
    let root = app.data.join("gravatar");
    tokio::fs::create_dir_all(&root).await?;
    if !tokio::fs::symlink_metadata(&root).await?.is_dir() {
        return Err(missing());
    }
    let path = root.join(hash);
    match tokio::fs::symlink_metadata(&path).await {
        Ok(metadata) if metadata.is_file() => {}
        Ok(_) => return Err(missing()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let bytes = app
                .network
                .get(
                    &format!("https://secure.gravatar.com/avatar/{hash}?s=96&d=identicon&r=g"),
                    false,
                    512 * 1024,
                    &app.shutdown,
                )
                .await
                .map_err(|_| missing())?;
            let staging = app.data.join(".media-staging");
            if !tokio::fs::symlink_metadata(&staging).await?.is_dir() {
                return Err(missing());
            }
            let temporary = tempfile::Builder::new()
                .prefix("gravatar-")
                .tempdir_in(staging)?;
            let source = temporary.path().join("image");
            tokio::fs::write(&source, bytes).await?;
            inspect(app, &source, &app.shutdown)
                .await
                .map_err(|_| missing())?;
            let _content = app.content_lock.write().await;
            tokio::fs::rename(source, &path).await?;
        }
        Err(error) => return Err(error.into()),
    }
    let mut header = [0u8; 32];
    let mut file = tokio::fs::File::open(&path).await?;
    let count = file.read(&mut header).await?;
    let format = image::guess_format(&header[..count]).map_err(|_| missing())?;
    let mut service = tower_http::services::ServeFile::new_with_mime(
        path,
        &format.to_mime_type().parse().unwrap(),
    );
    Ok(service
        .try_call(axum::http::Request::new(axum::body::Body::empty()))
        .await?
        .into_response())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{config::Config, database, network, tasks};
    use std::sync::{Arc, Mutex};
    #[tokio::test]
    async fn gravatar_validates_and_reuses_remote_images() {
        let temporary = tempfile::tempdir().unwrap();
        let data = temporary.path().join("data");
        let fixture = Arc::new(Mutex::new(network::TestResponses::default()));
        let mut network = network::Client::new(None);
        network.fixture = Some(fixture.clone());
        let app = App {
            db: database::open(&data).await.unwrap(),
            tasks: tasks::open(&data.join("runtime")).await.unwrap(),
            data,
            network,
            shutdown: CancellationToken::new(),
            config: Arc::new(Config::default()),
            password_slots: Arc::new(tokio::sync::Semaphore::new(2)),
            media_slots: Arc::new(tokio::sync::Semaphore::new(2)),
            content_lock: Arc::new(tokio::sync::RwLock::new(())),
        };
        recover(&app).await.unwrap();
        let picture = temporary.path().join("avatar.png");
        image::RgbImage::new(96, 96).save(&picture).unwrap();
        let bytes = std::fs::read(picture).unwrap();
        let hash = "0123456789abcdef0123456789abcdef";
        let url = format!("https://secure.gravatar.com/avatar/{hash}?s=96&d=identicon&r=g");
        fixture
            .lock()
            .unwrap()
            .responses
            .insert(url.clone(), bytes.clone());
        let mut requests = tokio::task::JoinSet::new();
        for _ in 0..6 {
            let app = app.clone();
            requests.spawn(async move {
                let response = gravatar(&app, hash).await.unwrap();
                assert_eq!(response.headers()["Content-Type"], "image/png");
                axum::body::to_bytes(response.into_body(), 512 * 1024)
                    .await
                    .unwrap()
            });
        }
        while let Some(result) = requests.join_next().await {
            assert_eq!(result.unwrap().as_ref(), bytes);
        }
        assert_eq!(fixture.lock().unwrap().requests, vec![(url, None)]);
        for invalid in ["../../private", "ABCDEF0123456789ABCDEF0123456789", "a", ""] {
            assert!(gravatar(&app, invalid).await.is_err());
        }
        assert_eq!(fixture.lock().unwrap().requests.len(), 1);
        let bad = "ffffffffffffffffffffffffffffffff";
        fixture.lock().unwrap().responses.insert(
            format!("https://secure.gravatar.com/avatar/{bad}?s=96&d=identicon&r=g"),
            b"<html>upstream error</html>".to_vec(),
        );
        assert!(gravatar(&app, bad).await.is_err());
        assert!(!app.data.join("gravatar").join(bad).exists());
        assert_eq!(
            std::fs::read_dir(app.data.join(".media-staging"))
                .unwrap()
                .count(),
            0
        );
        let private = temporary.path().join("private");
        std::fs::write(&private, b"private").unwrap();
        std::os::unix::fs::symlink(private, app.data.join("gravatar").join(bad)).unwrap();
        assert!(gravatar(&app, bad).await.is_err());
        assert_eq!(fixture.lock().unwrap().requests.len(), 2);
    }
}
