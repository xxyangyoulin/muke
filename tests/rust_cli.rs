use muke::tasks;
use serde_json::json;
use std::{fs::OpenOptions, process::Command};

#[tokio::test]
async fn offline_commands_respect_the_running_service_lock() {
    let root = tempfile::tempdir().unwrap();
    let data = root.path().join("data");
    let work = root.path().join("work");
    std::fs::create_dir(&data).unwrap();
    let config = root.path().join("muke.toml");
    let contents = toml::to_string(&json!({
        "data_dir": data,
        "export_dir": root.path().join("dist"),
        "frontend": concat!(env!("CARGO_MANIFEST_DIR"), "/frontend"),
    }))
    .unwrap();
    std::fs::write(&config, contents).unwrap();
    let pool = tasks::open(&work).await.unwrap();
    let id = tasks::submit(&pool, &tasks::Task::Export {}).await.unwrap();
    sqlx::query("UPDATE jobs SET state='running' WHERE id=?")
        .bind(id)
        .execute(&pool)
        .await
        .unwrap();
    let job_dir = work.join(format!("job-{id}"));
    std::fs::create_dir(&job_dir).unwrap();
    std::fs::write(job_dir.join("active"), "running").unwrap();
    let artifact = muke::export::artifact::Artifact::new(&root.path().join("dist")).unwrap();
    let abandoned = artifact.path().to_owned();
    std::mem::forget(artifact);
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(data.join("service.lock"))
        .unwrap();
    fs2::FileExt::lock_exclusive(&lock).unwrap();
    let backup = root.path().join("backup.zip");
    let run = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_muke"))
            .arg("--config")
            .arg(&config)
            .arg("--work-dir")
            .arg(&work)
            .args(args)
            .output()
            .unwrap()
    };
    for args in [
        vec!["export"],
        vec!["backup", backup.to_str().unwrap()],
        vec!["serve"],
    ] {
        let result = run(&args);
        assert!(!result.status.success());
        assert!(String::from_utf8_lossy(&result.stderr).contains("已有服务或维护命令运行"));
    }
    assert!(run(&["tasks"]).status.success());
    let state: String = sqlx::query_scalar("SELECT state FROM jobs WHERE id=?")
        .bind(id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(state, "running");
    assert_eq!(
        std::fs::read_to_string(job_dir.join("active")).unwrap(),
        "running"
    );
    assert!(!data.join("blog.sqlite").exists());
    assert!(!backup.exists());
    assert!(abandoned.exists());
    drop(lock);
    let result = run(&["export"]);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(root.path().join("dist/index.html").is_file());
    assert!(!abandoned.exists());
    let state: String = sqlx::query_scalar("SELECT state FROM jobs WHERE id=?")
        .bind(id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(state, "interrupted");
    assert!(!job_dir.exists());
}
