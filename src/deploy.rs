use crate::{api::App, config::Settings, tasks};
use anyhow::{Result, bail};
use serde_json::{Value, json};
use sha1::{Digest, Sha1};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
    time::Duration,
};
use tokio_util::sync::CancellationToken;

const API: &str = "https://api.netlify.com/api/v1";

/// Netlify API 请求：传输层失败与 5xx 重试最多 6 次（POST 不重试，避免重复创建部署）。
#[allow(clippy::too_many_arguments)]
async fn request(
    app: &App,
    token: &str,
    method: &str,
    address: &str,
    body: Option<Vec<u8>>,
    content_type: &str,
    timeout: Duration,
    stop: &CancellationToken,
) -> Result<(u16, Value)> {
    let headers = vec![
        ("Authorization".to_owned(), format!("Bearer {token}")),
        ("Content-Type".to_owned(), content_type.to_owned()),
    ];
    for attempt in 1u32.. {
        if stop.is_cancelled() {
            bail!("任务已取消");
        }
        let result = app
            .network
            .request(
                address,
                method,
                &headers,
                body.clone(),
                None,
                timeout,
                false,
                8 * 1024 * 1024,
                stop,
            )
            .await;
        match result {
            Ok((status, response)) => {
                if status < 500 || method == "POST" || attempt >= 6 {
                    let value = serde_json::from_slice(&response).unwrap_or_else(|_| json!({}));
                    return Ok((status, value));
                }
            }
            Err(error) if attempt >= 6 => bail!("无法连接 Netlify：{error}"),
            Err(_) => {}
        }
        for _ in 0..(attempt * 3).min(15) {
            if stop.is_cancelled() {
                bail!("任务已取消");
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    }
    unreachable!()
}

fn hashes(dist: &Path, stop: &CancellationToken) -> Result<BTreeMap<String, String>> {
    let mut files = BTreeMap::new();
    let mut directories = vec![dist.to_owned()];
    while let Some(directory) = directories.pop() {
        if stop.is_cancelled() {
            bail!("任务已取消");
        }
        for entry in std::fs::read_dir(&directory)? {
            let entry = entry?;
            let path = entry.path();
            if entry.file_type()?.is_dir() {
                directories.push(path);
            } else if entry.file_type()?.is_file() {
                let relative = path
                    .strip_prefix(dist)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/");
                files.insert(
                    format!("/{relative}"),
                    hex::encode(Sha1::digest(std::fs::read(&path)?)),
                );
            }
        }
    }
    Ok(files)
}

/// 摘要式增量部署：先导出，再按 Netlify required 清单上传缺失文件并等待发布。
/// 返回部署结果；取消时尽力取消远端部署。
pub async fn run(app: &App, ctx: &tasks::Context) -> Result<Value> {
    let settings = Settings::load(app).await?;
    let (token, site) = (
        settings.get("netlify_token").to_owned(),
        settings.get("netlify_site").to_owned(),
    );
    if token.is_empty() || site.is_empty() {
        bail!("请先填写并保存 Netlify 令牌和站点 ID");
    }
    let exported =
        crate::export::site::export(app, &app.config.frontend, &app.config.export_dir, ctx).await?;
    let mut deploy_id = String::new();
    let result = upload(app, ctx, &token, &site, &mut deploy_id).await;
    match result {
        Ok(url) => Ok(json!({
            "ok": true,
            "url": url,
            "posts": exported["posts"],
            "articles": exported["articles"],
            "plugins": exported["plugins"],
        })),
        Err(error) => {
            if ctx.stop.is_cancelled()
                && !deploy_id.is_empty()
                && !cancel_remote(app, &token, &deploy_id).await
            {
                bail!("本地部署已停止，远端取消未成功，请检查 Netlify 状态");
            }
            Err(error)
        }
    }
}

/// 取消远端部署；返回远端是否接受取消。
async fn cancel_remote(app: &App, token: &str, deploy_id: &str) -> bool {
    request(
        app,
        token,
        "POST",
        &format!("{API}/deploys/{deploy_id}/cancel"),
        None,
        "application/json",
        Duration::from_secs(15),
        &CancellationToken::new(),
    )
    .await
    .map(|(status, _)| status < 400)
    .unwrap_or(false)
}

async fn upload(
    app: &App,
    ctx: &tasks::Context,
    token: &str,
    site: &str,
    deploy_id: &mut String,
) -> Result<String> {
    let files = hashes(&app.config.export_dir, &ctx.stop)?;
    if files.is_empty() {
        bail!("导出目录为空，请先生成静态站点");
    }
    let manifest: serde_json::Map<String, Value> = files
        .iter()
        .map(|(path, sha1)| (path.clone(), Value::String(sha1.clone())))
        .collect();
    let (status, deploy) = request(
        app,
        token,
        "POST",
        &format!("{API}/sites/{site}/deploys"),
        Some(serde_json::to_vec(&json!({"files": manifest}))?),
        "application/json",
        Duration::from_secs(600),
        &ctx.stop,
    )
    .await?;
    if status >= 400 {
        bail!(
            "{}（HTTP {status}）",
            deploy["message"].as_str().unwrap_or("创建部署失败")
        );
    }
    *deploy_id = deploy["id"].as_str().unwrap_or("").to_owned();
    let required: BTreeSet<String> = deploy["required"]
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();
    let pending: Vec<&String> = files
        .iter()
        .filter(|(_, sha1)| required.contains(*sha1))
        .map(|(path, _)| path)
        .collect();
    if pending.is_empty() {
        ctx.progress("等待 Netlify 构建", 0, None).await?;
    } else {
        for (index, path) in pending.iter().enumerate() {
            ctx.progress(
                &format!("上传 {}", path.trim_matches('/')),
                index as i64,
                Some(pending.len() as i64),
            )
            .await?;
            let body =
                tokio::fs::read(app.config.export_dir.join(path.trim_start_matches('/'))).await?;
            let encoded = path
                .trim_start_matches('/')
                .split('/')
                .map(|segment| {
                    url::form_urlencoded::byte_serialize(segment.as_bytes()).collect::<String>()
                })
                .collect::<Vec<_>>()
                .join("/");
            // 超时按体积自适应：600s 基础 + 每 25KB/s 兜底上行速率另计
            let timeout = Duration::from_secs(600 + (body.len() as u64).div_ceil(25_000));
            let (status, _) = request(
                app,
                token,
                "PUT",
                &format!("{API}/deploys/{deploy_id}/files/{encoded}"),
                Some(body),
                "application/octet-stream",
                timeout,
                &ctx.stop,
            )
            .await?;
            if status >= 400 {
                bail!("上传 {path} 失败（HTTP {status}）");
            }
        }
        ctx.progress(
            "等待 Netlify 构建",
            pending.len() as i64,
            Some(pending.len() as i64),
        )
        .await?;
    }
    // 构建等待放宽到 10 分钟：数千文件的站点 Netlify 处理可能超过 5 分钟
    for _ in 0..200 {
        for _ in 0..3 {
            if ctx.stop.is_cancelled() {
                bail!("任务已取消");
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
        let (_, info) = request(
            app,
            token,
            "GET",
            &format!("{API}/deploys/{deploy_id}"),
            None,
            "application/json",
            Duration::from_secs(600),
            &ctx.stop,
        )
        .await?;
        match info["state"].as_str() {
            Some("ready") => {
                return Ok(info["ssl_url"]
                    .as_str()
                    .or_else(|| info["url"].as_str())
                    .unwrap_or_default()
                    .to_owned());
            }
            Some("error") => bail!("Netlify 处理部署失败，请到控制台查看日志"),
            _ => {}
        }
    }
    bail!("等待 Netlify 发布超时，请稍后到控制台查看结果")
}

/// 新动态触碰自动部署防抖：开关开启且凭据齐备时顺延到期时间（编辑不触发，由调用方保证）。
pub async fn touch(app: &App) -> Result<()> {
    let settings = Settings::load(app).await?;
    if !settings.enabled("auto_deploy")
        || settings.get("netlify_token").is_empty()
        || settings.get("netlify_site").is_empty()
    {
        return Ok(());
    }
    let minutes = settings.number("auto_deploy_debounce").clamp(1, 1440);
    sqlx::query("INSERT INTO deferred(key,due_at) VALUES('auto-deploy',?) ON CONFLICT(key) DO UPDATE SET due_at=excluded.due_at")
        .bind(chrono::Utc::now().timestamp() + minutes * 60)
        .execute(&app.tasks)
        .await?;
    Ok(())
}

/// 取消排队中的自动部署（手动部署发起或关闭开关时）。
pub async fn clear_pending(app: &App) -> Result<()> {
    sqlx::query("DELETE FROM deferred WHERE key='auto-deploy'")
        .execute(&app.tasks)
        .await?;
    Ok(())
}

/// 到期检查：由任务调度循环周期调用，到期即提交部署任务（去重由活动任务机制保证）。
pub async fn due(pool: &sqlx::SqlitePool) -> Result<bool> {
    let now = chrono::Utc::now().timestamp();
    let due = sqlx::query_scalar::<_, i64>(
        "DELETE FROM deferred WHERE key='auto-deploy' AND due_at<=? RETURNING 1",
    )
    .bind(now)
    .fetch_optional(pool)
    .await?;
    Ok(due.is_some())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{config::Config, database, network, plugins, tasks};
    use serde_json::json;
    use std::sync::{Arc, Mutex};

    async fn app(root: &Path, fixture: Arc<Mutex<network::TestResponses>>) -> App {
        let data = root.join("data");
        let mut client = network::Client::new(None);
        client.fixture = Some(fixture);
        let config = Config {
            data_dir: data.clone(),
            export_dir: root.join("dist"),
            frontend: Path::new(env!("CARGO_MANIFEST_DIR")).join("frontend"),
            ..Config::default()
        };
        let app = App {
            db: database::open(&data).await.unwrap(),
            tasks: tasks::open(&root.join("work")).await.unwrap(),
            network: client,
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

    async fn context(app: &App) -> tasks::Context {
        sqlx::query("INSERT OR IGNORE INTO jobs(id,kind,resource,dedup,payload,state) VALUES(1,'deploy','export','fixture','{}','running')")
            .execute(&app.tasks)
            .await
            .unwrap();
        tasks::Context {
            id: 1,
            pool: app.tasks.clone(),
            directory: app.config.export_dir.parent().unwrap().join("job-1"),
            stop: CancellationToken::new(),
        }
    }

    fn sha1_hex(data: &[u8]) -> String {
        hex::encode(Sha1::digest(data))
    }

    async fn seeded(app: &App) {
        for (key, value) in [
            ("netlify_token", "secret-token"),
            ("netlify_site", "test-site"),
        ] {
            sqlx::query("INSERT INTO settings VALUES(?,?)")
                .bind(key)
                .bind(value)
                .execute(&app.db)
                .await
                .unwrap();
        }
    }

    #[tokio::test]
    async fn upload_sends_only_required_files_and_waits_for_ready() {
        let root = tempfile::tempdir().unwrap();
        let dist = root.path().join("dist");
        std::fs::create_dir_all(dist.join("assets")).unwrap();
        std::fs::write(dist.join("index.html"), b"hello dist").unwrap();
        std::fs::write(dist.join("assets/style.css"), b"body{}").unwrap();
        let fixture = Arc::new(Mutex::new(network::TestResponses::default()));
        let app = app(root.path(), fixture.clone()).await;
        seeded(&app).await;
        let required = sha1_hex(b"hello dist");
        {
            let mut responses = fixture.lock().unwrap();
            responses.responses.insert(
                "https://api.netlify.com/api/v1/sites/test-site/deploys".into(),
                json!({"id":"dep1","required":[required]})
                    .to_string()
                    .into_bytes(),
            );
            responses.responses.insert(
                "https://api.netlify.com/api/v1/deploys/dep1/files/index.html".into(),
                b"{}".to_vec(),
            );
            responses.responses.insert(
                "https://api.netlify.com/api/v1/deploys/dep1".into(),
                json!({"state":"ready","ssl_url":"https://muke-test.netlify.app"})
                    .to_string()
                    .into_bytes(),
            );
        }
        let ctx = context(&app).await;
        let mut deploy_id = String::new();
        let url = upload(&app, &ctx, "secret-token", "test-site", &mut deploy_id)
            .await
            .unwrap();
        assert_eq!(url, "https://muke-test.netlify.app");
        assert_eq!(deploy_id, "dep1");
        let requests = fixture.lock().unwrap().requests.clone();
        assert_eq!(requests.len(), 3);
        assert_eq!(requests[0].1.as_deref(), Some("POST"));
        assert_eq!(
            requests[1].0.as_str(),
            "https://api.netlify.com/api/v1/deploys/dep1/files/index.html"
        );
        assert_eq!(requests[1].1.as_deref(), Some("PUT"));
        assert!(!requests.iter().any(|(url, _)| url.contains("style.css")));
        // 进度阶段落在任务行
        let phase: String = sqlx::query_scalar("SELECT phase FROM jobs WHERE id=1")
            .fetch_one(&app.tasks)
            .await
            .unwrap();
        assert_eq!(phase, "等待 Netlify 构建");
    }

    #[tokio::test]
    async fn empty_required_skips_upload_and_failures_propagate() {
        let root = tempfile::tempdir().unwrap();
        let dist = root.path().join("dist");
        std::fs::create_dir_all(&dist).unwrap();
        std::fs::write(dist.join("index.html"), b"hello").unwrap();
        let fixture = Arc::new(Mutex::new(network::TestResponses::default()));
        let app = app(root.path(), fixture.clone()).await;
        seeded(&app).await;
        {
            let mut responses = fixture.lock().unwrap();
            responses.responses.insert(
                "https://api.netlify.com/api/v1/sites/test-site/deploys".into(),
                json!({"id":"dep2","required":[]}).to_string().into_bytes(),
            );
            responses.responses.insert(
                "https://api.netlify.com/api/v1/deploys/dep2".into(),
                json!({"state":"ready","url":"https://example.netlify.app"})
                    .to_string()
                    .into_bytes(),
            );
        }
        let ctx = context(&app).await;
        let mut deploy_id = String::new();
        let url = upload(&app, &ctx, "secret-token", "test-site", &mut deploy_id)
            .await
            .unwrap();
        assert_eq!(url, "https://example.netlify.app");
        assert_eq!(fixture.lock().unwrap().requests.len(), 2);
        // 创建失败信息透传
        {
            let mut responses = fixture.lock().unwrap();
            responses.responses.insert(
                "https://api.netlify.com/api/v1/sites/test-site/deploys".into(),
                json!({"message":"Unauthorized"}).to_string().into_bytes(),
            );
            responses.statuses.insert(
                "https://api.netlify.com/api/v1/sites/test-site/deploys".into(),
                401,
            );
        }
        let mut deploy_id = String::new();
        let error = upload(&app, &ctx, "bad-token", "test-site", &mut deploy_id)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("Unauthorized"));
        // 空目录与取消
        std::fs::remove_file(dist.join("index.html")).unwrap();
        let mut deploy_id = String::new();
        assert!(
            upload(&app, &ctx, "secret-token", "test-site", &mut deploy_id)
                .await
                .unwrap_err()
                .to_string()
                .contains("导出目录为空")
        );
        let cancelled = CancellationToken::new();
        cancelled.cancel();
        let ctx = tasks::Context {
            stop: cancelled,
            ..context(&app).await
        };
        std::fs::write(dist.join("index.html"), b"hello").unwrap();
        let mut deploy_id = String::new();
        assert!(
            upload(&app, &ctx, "secret-token", "test-site", &mut deploy_id)
                .await
                .unwrap_err()
                .to_string()
                .contains("任务已取消")
        );
    }

    #[tokio::test]
    async fn remote_cancel_reports_outcome() {
        let root = tempfile::tempdir().unwrap();
        let fixture = Arc::new(Mutex::new(network::TestResponses::default()));
        let app = app(root.path(), fixture.clone()).await;
        fixture.lock().unwrap().responses.insert(
            "https://api.netlify.com/api/v1/deploys/dep9/cancel".into(),
            b"{}".to_vec(),
        );
        assert!(cancel_remote(&app, "token", "dep9").await);
        {
            let mut responses = fixture.lock().unwrap();
            responses.statuses.insert(
                "https://api.netlify.com/api/v1/deploys/dep9/cancel".into(),
                500,
            );
        }
        // 5xx POST 不重试，直接返回失败
        assert!(!cancel_remote(&app, "token", "dep9").await);
    }

    #[tokio::test]
    async fn deferred_touch_extends_and_due_fires_once() {
        let root = tempfile::tempdir().unwrap();
        let fixture = Arc::new(Mutex::new(network::TestResponses::default()));
        let app = app(root.path(), fixture).await;
        // 未开启或凭据缺失时不排队
        super::touch(&app).await.unwrap();
        assert!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM deferred")
                .fetch_one(&app.tasks)
                .await
                .unwrap()
                == 0
        );
        seeded(&app).await;
        sqlx::query("INSERT INTO settings VALUES('auto_deploy','1')")
            .execute(&app.db)
            .await
            .unwrap();
        sqlx::query("INSERT INTO settings VALUES('auto_deploy_debounce','5')")
            .execute(&app.db)
            .await
            .unwrap();
        super::touch(&app).await.unwrap();
        let first: i64 = sqlx::query_scalar("SELECT due_at FROM deferred WHERE key='auto-deploy'")
            .fetch_one(&app.tasks)
            .await
            .unwrap();
        assert!(first > chrono::Utc::now().timestamp() + 240);
        // 再次触碰顺延
        sqlx::query("UPDATE deferred SET due_at=1 WHERE key='auto-deploy'")
            .execute(&app.tasks)
            .await
            .unwrap();
        super::touch(&app).await.unwrap();
        let second: i64 = sqlx::query_scalar("SELECT due_at FROM deferred WHERE key='auto-deploy'")
            .fetch_one(&app.tasks)
            .await
            .unwrap();
        assert!(second > first - 300);
        // 到期触发一次后清空
        sqlx::query("UPDATE deferred SET due_at=1 WHERE key='auto-deploy'")
            .execute(&app.tasks)
            .await
            .unwrap();
        assert!(due(&app.tasks).await.unwrap());
        assert!(!due(&app.tasks).await.unwrap());
        super::touch(&app).await.unwrap();
        super::clear_pending(&app).await.unwrap();
        assert!(!due(&app.tasks).await.unwrap());
    }
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn deploy_task_runs_export_upload_and_result_through_the_runner() {
        let root = tempfile::tempdir().unwrap();
        let fixture = Arc::new(Mutex::new(network::TestResponses::default()));
        let app = app(root.path(), fixture.clone()).await;
        seeded(&app).await;
        sqlx::query("INSERT INTO posts(content,source,images,meta,created_at,updated_at) VALUES('动态','','[]','{}','2026-11-26T08:00:00+08:00','2026-11-26T08:00:00+08:00')")
            .execute(&app.db)
            .await
            .unwrap();
        {
            let mut responses = fixture.lock().unwrap();
            responses.responses.insert(
                "https://api.netlify.com/api/v1/sites/test-site/deploys".into(),
                br#"{"id":"dep1","required":[]}"#.to_vec(),
            );
            responses.responses.insert(
                "https://api.netlify.com/api/v1/deploys/dep1".into(),
                br#"{"state":"ready","ssl_url":"https://muke-test.netlify.app"}"#.to_vec(),
            );
        }
        let stop = CancellationToken::new();
        let runner = {
            let app = app.clone();
            let work = app.data.parent().unwrap().join("work");
            let stop = stop.clone();
            tokio::spawn(async move { tasks::run(app, work, stop).await })
        };
        let id = tasks::submit(&app.tasks, &tasks::Task::Deploy {})
            .await
            .unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        let job = loop {
            let job = tasks::list(&app.tasks)
                .await
                .unwrap()
                .as_array()
                .unwrap()
                .iter()
                .find(|job| job["id"].as_i64() == Some(id))
                .cloned()
                .unwrap();
            if !matches!(job["state"].as_str(), Some("queued" | "running")) {
                break job;
            }
            assert!(std::time::Instant::now() < deadline, "部署任务超时");
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        };
        assert_eq!(job["state"], "succeeded");
        assert_eq!(job["result"]["url"], "https://muke-test.netlify.app");
        assert_eq!(job["result"]["posts"], 1);
        assert!(app.config.export_dir.join("index.html").is_file());
        stop.cancel();
        runner.await.unwrap().unwrap();
    }
}
