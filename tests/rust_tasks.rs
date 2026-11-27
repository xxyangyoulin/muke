use muke::{render, tasks};
use serde_json::json;
use std::sync::Arc;

#[tokio::test]
async fn duplicate_submissions_have_one_active_job() {
    let root = tempfile::tempdir().unwrap();
    let data = root.path().join("data");
    let work = root.path().join("work");
    std::fs::create_dir_all(data.join("uploads")).unwrap();
    let source = data.join("uploads/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa.png");
    std::fs::write(&source, b"fixture").unwrap();
    let pool = tasks::open(&work).await.unwrap();
    let mut jobs = tokio::task::JoinSet::new();
    for _ in 0..12 {
        let pool = pool.clone();
        let data = data.clone();
        let source = source.clone();
        jobs.spawn(async move { tasks::enqueue(&pool, &data, &source).await.unwrap() });
    }
    while let Some(job) = jobs.join_next().await {
        assert_eq!(job.unwrap(), 1);
    }
    assert_eq!(
        tasks::list(&pool).await.unwrap().as_array().unwrap().len(),
        1
    );
    assert!(tasks::cancel(&pool, 1).await.unwrap());
    assert!(!tasks::cancel(&pool, 1).await.unwrap());
    assert!(tasks::enqueue(&pool, &data, &source).await.unwrap() > 1);
}

#[tokio::test]
async fn export_task_publishes_dist_through_the_runner() {
    use muke::{api::App, config::Config, database, network, plugins};
    use tokio_util::sync::CancellationToken;
    let root = tempfile::tempdir().unwrap();
    let data = root.path().join("data");
    let work = root.path().join("work");
    let config = Config {
        data_dir: data.clone(),
        export_dir: root.path().join("site"),
        frontend: std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("frontend"),
        ..Config::default()
    };
    let app = App {
        plugins: muke::plugins::Registry::default(),
        db: database::open(&data).await.unwrap(),
        tasks: tasks::open(&work).await.unwrap(),
        network: network::Client::new(None),
        data,
        config: Arc::new(config),
        shutdown: CancellationToken::new(),
        password_slots: Arc::new(tokio::sync::Semaphore::new(2)),
        media_slots: Arc::new(tokio::sync::Semaphore::new(2)),
        content_lock: Arc::new(tokio::sync::RwLock::new(())),
    };
    plugins::migrate(&app).await.unwrap();
    sqlx::query("INSERT INTO posts(content,source,images,meta,created_at,updated_at) VALUES('动态','','[]','{}','2026-11-26T08:00:00+08:00','2026-11-26T08:00:00+08:00')")
        .execute(&app.db)
        .await
        .unwrap();
    sqlx::query("INSERT INTO articles(title,content,format,created_at,updated_at) VALUES('文章','正文','html','2026-11-26T08:00:00+08:00','2026-11-26T08:00:00+08:00')")
        .execute(&app.db)
        .await
        .unwrap();
    let stop = CancellationToken::new();
    let runner = {
        let app = app.clone();
        let work = work.clone();
        let stop = stop.clone();
        tokio::spawn(async move { tasks::run(app, work, stop).await })
    };
    let first = tasks::submit(&app.tasks, &tasks::Task::Export {})
        .await
        .unwrap();
    // 去重：活动任务重复提交返回同一编号
    assert_eq!(
        tasks::submit(&app.tasks, &tasks::Task::Export {})
            .await
            .unwrap(),
        first
    );
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    let job = loop {
        let job = tasks::list(&app.tasks)
            .await
            .unwrap()
            .as_array()
            .unwrap()
            .iter()
            .find(|job| job["id"].as_i64() == Some(first))
            .cloned()
            .unwrap();
        if !matches!(job["state"].as_str(), Some("queued" | "running")) {
            break job;
        }
        assert!(std::time::Instant::now() < deadline, "导出任务超时");
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    };
    assert_eq!(job["state"], "succeeded");
    assert_eq!(job["result"]["posts"], 1);
    assert_eq!(job["result"]["articles"], 1);
    let dist = app.config.export_dir.clone();
    assert!(dist.join("index.html").is_file());
    assert!(dist.join("rss.xml").is_file());
    assert_eq!(
        std::fs::read_dir(root.path())
            .unwrap()
            .filter_map(std::result::Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().starts_with(".export-"))
            .count(),
        0
    );
    // 完成后可再次提交
    assert!(
        tasks::submit(&app.tasks, &tasks::Task::Export {})
            .await
            .unwrap()
            > first
    );
    stop.cancel();
    runner.await.unwrap().unwrap();
}

#[tokio::test]
async fn uploads_cannot_escape_through_symlinks() {
    let root = tempfile::tempdir().unwrap();
    let data = root.path().join("data");
    std::fs::create_dir_all(data.join("uploads")).unwrap();
    let outside = root.path().join("private");
    std::fs::write(&outside, "private").unwrap();
    let link = data.join("uploads/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa.png");
    std::os::unix::fs::symlink(outside, &link).unwrap();
    let pool = tasks::open(&root.path().join("work")).await.unwrap();
    assert!(tasks::enqueue(&pool, &data, &link).await.is_err());
}

#[test]
fn markdown_raw_html_is_text_and_rich_html_is_sanitized() {
    let markdown = render::article(
        &json!({"format":"md","content":"<script>alert(1)</script>\n\n[link](javascript:alert(1))"}),
    );
    assert!(!markdown.contains("<script>"));
    assert!(!markdown.contains("href=\"javascript:"));
    let rich = render::article(
        &json!({"format":"html","content":"<p>你好</p><script>alert(1)</script><img src=x onerror=alert(1)>"}),
    );
    assert!(rich.contains("你好"));
    assert!(!rich.contains("onerror"));
    assert!(!rich.contains("<script>"));
}
