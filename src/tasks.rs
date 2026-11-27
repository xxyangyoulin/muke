use crate::{api::App, auth::Session, error::Error};
use anyhow::{Context as _, Result, bail};
use axum::{
    extract::{Extension, State},
    response::{
        Sse,
        sse::{Event, KeepAlive},
    },
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::{Connection, SqliteConnection, SqlitePool, sqlite::SqliteConnectOptions};
use std::{
    convert::Infallible,
    path::{Path, PathBuf},
    time::Duration,
};
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;

#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Task {
    Image {
        source: String,
    },
    PluginSync {
        plugin: String,
        params: Value,
    },
    PluginInstall {
        source_kind: String,
        source: String,
        #[serde(default)]
        path: String,
    },
    PluginDiscover {
        source_kind: String,
        source: String,
    },
    Export {},
    Deploy {},
    Backup {},
}
impl Task {
    fn kind(&self) -> &'static str {
        match self {
            Self::Image { .. } => "image",
            Self::PluginSync { .. } => "plugin_sync",
            Self::PluginInstall { .. } => "plugin_install",
            Self::PluginDiscover { .. } => "plugin_discover",
            Self::Export { .. } => "export",
            Self::Deploy { .. } => "deploy",
            Self::Backup { .. } => "backup",
        }
    }
    fn resource(&self) -> String {
        match self {
            Self::Image { source } => format!("image:{source}"),
            Self::PluginSync { plugin, .. } => format!("plugin:{plugin}"),
            Self::PluginInstall { .. } | Self::PluginDiscover { .. } => "plugin_install".into(),
            // 导出与部署共写同一产物目录，必须串行
            Self::Export { .. } | Self::Deploy { .. } => "export".into(),
            Self::Backup { .. } => "backup".into(),
        }
    }
}

#[derive(Clone)]
pub struct Context {
    pub id: i64,
    pub pool: SqlitePool,
    pub directory: PathBuf,
    pub stop: CancellationToken,
}
impl Context {
    pub async fn log(&self, text: &str) -> Result<()> {
        sqlx::query("INSERT INTO job_logs(job_id,output) VALUES(?,substr(?,-65536)) ON CONFLICT(job_id) DO UPDATE SET output=substr(output||excluded.output,-65536)")
            .bind(self.id).bind(text).execute(&self.pool).await?;
        Ok(())
    }
    pub fn check(&self) -> Result<()> {
        if self.stop.is_cancelled() {
            bail!("任务已取消");
        }
        Ok(())
    }
    pub async fn progress(&self, phase: &str, completed: i64, total: Option<i64>) -> Result<()> {
        self.check()?;
        sqlx::query("UPDATE jobs SET phase=?,completed=?,total=? WHERE id=? AND state='running'")
            .bind(phase)
            .bind(completed)
            .bind(total)
            .bind(self.id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}

pub async fn open(directory: &Path) -> Result<SqlitePool> {
    tokio::fs::create_dir_all(directory).await?;
    let options = SqliteConnectOptions::new()
        .filename(directory.join("tasks.sqlite"))
        .create_if_missing(true)
        .busy_timeout(Duration::from_secs(5));
    let mut connection = SqliteConnection::connect_with(&options).await?;
    let mut tx = connection.begin().await?;
    sqlx::query("CREATE TABLE IF NOT EXISTS jobs (id INTEGER PRIMARY KEY AUTOINCREMENT, kind TEXT NOT NULL, resource TEXT NOT NULL, dedup TEXT NOT NULL, payload TEXT NOT NULL, state TEXT NOT NULL DEFAULT 'queued', cancel_requested INTEGER NOT NULL DEFAULT 0, phase TEXT NOT NULL DEFAULT '', completed INTEGER NOT NULL DEFAULT 0, total INTEGER, result TEXT, error TEXT, created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP, started_at TEXT, finished_at TEXT)").execute(&mut *tx).await?;
    sqlx::query("CREATE UNIQUE INDEX IF NOT EXISTS jobs_active ON jobs(dedup) WHERE state IN ('queued','running')").execute(&mut *tx).await?;
    sqlx::query("CREATE TABLE IF NOT EXISTS schedules (key TEXT PRIMARY KEY, payload TEXT NOT NULL, interval_seconds INTEGER NOT NULL, next_run INTEGER NOT NULL)").execute(&mut *tx).await?;
    sqlx::query("CREATE TABLE IF NOT EXISTS job_logs (job_id INTEGER PRIMARY KEY REFERENCES jobs(id), output TEXT NOT NULL)").execute(&mut *tx).await?;
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS deferred (key TEXT PRIMARY KEY, due_at INTEGER NOT NULL)",
    )
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    connection.close().await?;
    Ok(sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(4)
        .connect_with(options)
        .await?)
}
pub async fn enqueue(pool: &SqlitePool, data: &Path, source: &Path) -> Result<i64> {
    let source = tokio::fs::canonicalize(source).await?;
    let uploads = tokio::fs::canonicalize(data.join("uploads")).await?;
    if !source.starts_with(uploads) || !source.is_file() {
        bail!("图片必须位于内容副本的 uploads 目录内");
    }
    let name = source.file_name().and_then(|s| s.to_str()).unwrap_or("");
    if !crate::api::upload_name(name) {
        bail!("图片文件名不符合上传引用规则");
    }
    submit(
        pool,
        &Task::Image {
            source: source.to_str().context("图片路径必须是 UTF-8")?.into(),
        },
    )
    .await
}
pub async fn submit(pool: &SqlitePool, task: &Task) -> Result<i64> {
    let payload = serde_json::to_string(task)?;
    Ok(sqlx::query_scalar("INSERT INTO jobs(kind,resource,dedup,payload) VALUES(?,?,?,?) ON CONFLICT(dedup) WHERE state IN ('queued','running') DO UPDATE SET dedup=excluded.dedup RETURNING id")
        .bind(task.kind()).bind(task.resource()).bind(crate::auth::digest(&payload)).bind(payload).fetch_one(pool).await?)
}
pub async fn list(pool: &SqlitePool) -> Result<Value> {
    let rows = sqlx::query("SELECT id,kind,state,cancel_requested,phase,completed,total,result,error,created_at,started_at,finished_at FROM jobs ORDER BY id DESC LIMIT 100").fetch_all(pool).await?;
    let mut jobs: Vec<Value> = rows.into_iter().map(crate::api::row_json).collect();
    for job in &mut jobs {
        job["cancel_requested"] = json!(job["cancel_requested"] == 1);
        job["result"] = job["result"]
            .as_str()
            .and_then(|s| serde_json::from_str(s).ok())
            .unwrap_or(Value::Null);
    }
    Ok(json!(jobs))
}
pub async fn cancel(pool: &SqlitePool, id: i64) -> Result<bool> {
    Ok(sqlx::query("UPDATE jobs SET cancel_requested=1,state=CASE WHEN state='queued' THEN 'cancelled' ELSE state END,finished_at=CASE WHEN state='queued' THEN CURRENT_TIMESTAMP ELSE finished_at END WHERE id=? AND state IN ('queued','running')").bind(id).execute(pool).await?.rows_affected()>0)
}
fn directory(root: &Path, id: i64, kind: &str) -> PathBuf {
    root.join(format!(
        "{}-{id}",
        if kind == "image" { "image" } else { "job" }
    ))
}
pub async fn recover(pool: &SqlitePool, root: &Path) -> Result<()> {
    let jobs: Vec<(i64, String, i64)> =
        sqlx::query_as("SELECT id,kind,cancel_requested FROM jobs WHERE state='running'")
            .fetch_all(pool)
            .await?;
    for (id, kind, cancel) in jobs {
        let output = directory(root, id, &kind);
        if output.exists() {
            tokio::fs::remove_dir_all(output).await?;
        }
        sqlx::query("UPDATE jobs SET state=?,error=?,finished_at=CURRENT_TIMESTAMP WHERE id=?")
            .bind(if cancel != 0 {
                "cancelled"
            } else {
                "interrupted"
            })
            .bind("上次执行中断；未完成的临时文件已清理")
            .bind(id)
            .execute(pool)
            .await?;
    }
    Ok(())
}
pub struct Schedule {
    pub key: String,
    pub task: Task,
    pub interval: i64,
}
async fn schedule(app: &App) -> Result<()> {
    let mut schedules = crate::plugins::schedules(app).await?;
    let settings = crate::config::Settings::load(app).await?;
    if settings.enabled("backup_enabled") {
        schedules.push(Schedule {
            key: "backup".into(),
            task: Task::Backup {},
            interval: settings.number("backup_interval_hours").max(1) * 3600,
        });
    }
    let keys: Vec<_> = schedules.iter().map(|s| &s.key).collect();
    let now = chrono::Utc::now().timestamp();
    let mut tx = app.tasks.begin_with("BEGIN IMMEDIATE").await?;
    sqlx::query("DELETE FROM schedules WHERE key NOT IN (SELECT value FROM json_each(?))")
        .bind(json!(keys).to_string())
        .execute(&mut *tx)
        .await?;
    for schedule in schedules {
        let payload = serde_json::to_string(&schedule.task)?;
        sqlx::query("INSERT INTO schedules(key,payload,interval_seconds,next_run) VALUES(?,?,?,?) ON CONFLICT(key) DO UPDATE SET payload=excluded.payload,interval_seconds=excluded.interval_seconds,next_run=CASE WHEN interval_seconds<>excluded.interval_seconds THEN MIN(next_run,excluded.next_run) ELSE next_run END")
            .bind(&schedule.key).bind(&payload).bind(schedule.interval).bind(now+schedule.interval).execute(&mut *tx).await?;
        let due: bool = sqlx::query_scalar("SELECT next_run<=? FROM schedules WHERE key=?")
            .bind(now)
            .bind(&schedule.key)
            .fetch_one(&mut *tx)
            .await?;
        if due {
            sqlx::query("INSERT INTO jobs(kind,resource,dedup,payload) VALUES(?,?,?,?) ON CONFLICT(dedup) WHERE state IN ('queued','running') DO NOTHING")
                .bind(schedule.task.kind()).bind(schedule.task.resource()).bind(crate::auth::digest(&payload)).bind(payload).execute(&mut *tx).await?;
            sqlx::query("UPDATE schedules SET next_run=? WHERE key=?")
                .bind(now + schedule.interval)
                .bind(&schedule.key)
                .execute(&mut *tx)
                .await?;
        }
    }
    tx.commit().await?;
    Ok(())
}
pub async fn run(app: App, root: PathBuf, stop: CancellationToken) -> Result<()> {
    sqlx::query("UPDATE llm_calls SET status='interrupted' WHERE status='running'")
        .execute(&app.db)
        .await?;
    let mut jobs = JoinSet::new();
    let mut schedule_at = tokio::time::Instant::now();
    loop {
        while let Some(result) = jobs.try_join_next() {
            result??;
        }
        if stop.is_cancelled() {
            break;
        }
        if tokio::time::Instant::now() >= schedule_at {
            schedule(&app).await?;
            if crate::deploy::due(&app.tasks).await? {
                submit(&app.tasks, &Task::Deploy {}).await?;
            }
            schedule_at = tokio::time::Instant::now() + Duration::from_secs(5);
        }
        if jobs.len() < 4 {
            let next: Option<(i64,String)> = sqlx::query_as("UPDATE jobs SET state='running',started_at=CURRENT_TIMESTAMP WHERE id=(SELECT q.id FROM jobs q WHERE q.state='queued' AND NOT EXISTS(SELECT 1 FROM jobs r WHERE r.state='running' AND r.resource=q.resource) AND (q.kind<>'image' OR (SELECT COUNT(*) FROM jobs WHERE state='running' AND kind='image')<2) ORDER BY q.id LIMIT 1) RETURNING id,payload").fetch_optional(&app.tasks).await?;
            if let Some((id, payload)) = next {
                let app = app.clone();
                let root = root.clone();
                let stop = stop.clone();
                jobs.spawn(async move { execute(&app, &root, id, &payload, &stop).await });
                continue;
            }
        }
        tokio::select! {_=stop.cancelled()=>break, _=tokio::time::sleep(Duration::from_millis(100))=>{}}
    }
    while let Some(result) = jobs.join_next().await {
        result??;
    }
    Ok(())
}
async fn execute(
    app: &App,
    root: &Path,
    id: i64,
    payload: &str,
    stop: &CancellationToken,
) -> Result<()> {
    let task: Task = serde_json::from_str(payload)?;
    let image = matches!(task, Task::Image { .. });
    let ctx = Context {
        id,
        pool: app.tasks.clone(),
        directory: directory(root, id, task.kind()),
        stop: stop.child_token(),
    };
    let monitor_stop = ctx.stop.clone();
    let pool = app.tasks.clone();
    let monitor = tokio::spawn(async move {
        loop {
            tokio::select! {_=monitor_stop.cancelled()=>break, _=tokio::time::sleep(Duration::from_millis(100))=>{}}
            match sqlx::query_scalar::<_, i64>("SELECT cancel_requested FROM jobs WHERE id=?")
                .bind(id)
                .fetch_one(&pool)
                .await
            {
                Ok(0) => {}
                _ => {
                    monitor_stop.cancel();
                    break;
                }
            }
        }
    });
    let result = async {
        tokio::fs::create_dir_all(&ctx.directory).await?;
        ctx.check()?;
        match task {
            Task::Image{source}=> {
                let _permit = tokio::select!{_=ctx.stop.cancelled()=>bail!("任务已取消"), permit=app.media_slots.acquire()=>permit?};
                ctx.progress("压缩原图",0,Some(2)).await?;
                crate::media::compress(&source,&ctx.directory.join("image.webp"),100*1024,1600,&ctx.stop).await?;
                ctx.progress("生成缩略图",1,Some(2)).await?;
                crate::media::compress(&source,&ctx.directory.join("thumb.webp"),50*1024,480,&ctx.stop).await?;
                ctx.progress("完成",2,Some(2)).await?;
                Ok(json!({"images":2}))
            }
            Task::PluginSync{plugin,params}=>crate::plugins::run(app,&ctx,&plugin,params).await,
            Task::PluginInstall{source_kind,source,path}=>crate::plugins::install::run(app,&ctx,&source_kind,&source,&path).await,
            Task::PluginDiscover{source_kind,source}=>crate::plugins::install::discover(&ctx,&source_kind,&source).await,
            Task::Export{}=>crate::export::site::export(app,&app.config.frontend,&app.config.export_dir,&ctx).await,
            Task::Deploy{}=>crate::deploy::run(app,&ctx).await,
            Task::Backup{}=>crate::backup::run(app,&ctx).await,
        }
    }.await;
    monitor.abort();
    let _ = monitor.await;
    finish(&ctx, result, image).await
}

async fn finish(ctx: &Context, result: Result<Value>, image: bool) -> Result<()> {
    let mut tx = ctx.pool.begin_with("BEGIN IMMEDIATE").await?;
    let requested: i64 = sqlx::query_scalar("SELECT cancel_requested FROM jobs WHERE id=?")
        .bind(ctx.id)
        .fetch_one(&mut *tx)
        .await?;
    // A successful executor has finished its commit; a late request cannot undo it.
    let state = if result.is_ok() {
        "succeeded"
    } else if ctx.stop.is_cancelled() || requested != 0 {
        "cancelled"
    } else {
        "failed"
    };
    if (state != "succeeded" || !image) && ctx.directory.exists() {
        tokio::fs::remove_dir_all(&ctx.directory).await?;
    }
    let (value, error) = match result {
        Ok(value) => (Some(value.to_string()), None),
        Err(error) => (None, Some(error.to_string())),
    };
    sqlx::query(
        "UPDATE jobs SET state=?,result=?,error=?,finished_at=CURRENT_TIMESTAMP WHERE id=?",
    )
    .bind(state)
    .bind(value)
    .bind(error)
    .bind(ctx.id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}

pub async fn events(
    State(app): State<App>,
    Extension(session): Extension<Session>,
) -> crate::error::Result<axum::response::Response> {
    use axum::response::IntoResponse;
    session.require_admin()?;
    let stream = async_stream::stream! {
        let mut previous=String::new();
        let mut tick=tokio::time::interval(Duration::from_secs(1));
        loop {
            tokio::select!{_=app.shutdown.cancelled()=>break, _=tick.tick()=>{}}
            let valid=sqlx::query_scalar::<_,bool>("SELECT EXISTS(SELECT 1 FROM sessions WHERE id=? AND user_id IS NOT NULL AND expires_at>?)").bind(&session.id).bind(chrono::Utc::now().timestamp()).fetch_one(&app.db).await.unwrap_or(false);
            if !valid {break;}
            match list(&app.tasks).await {
                Ok(jobs)=> {
                    let data=json!({"jobs":jobs}).to_string();
                    if data!=previous { previous=data.clone(); yield Ok::<_,Infallible>(Event::default().event("tasks").data(data)); }
                }
                Err(_)=>break,
            }
        }
    };
    Ok(Sse::new(stream)
        .keep_alive(KeepAlive::default())
        .into_response())
}
pub fn api_error(error: anyhow::Error) -> Error {
    Error::Internal(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::export::artifact::Artifact;

    #[tokio::test]
    async fn cancellation_at_publication_boundary_records_the_actual_outcome() {
        let root = tempfile::tempdir().unwrap();
        let pool = open(&root.path().join("work")).await.unwrap();
        let destination = root.path().join("dist");
        std::fs::create_dir(&destination).unwrap();
        std::fs::write(destination.join("index.html"), "old").unwrap();
        for (timing, expected) in [
            ("before", "cancelled"),
            ("after-request", "succeeded"),
            ("after-token", "succeeded"),
        ] {
            let id = submit(&pool, &Task::Export {}).await.unwrap();
            sqlx::query("UPDATE jobs SET state='running' WHERE id=?")
                .bind(id)
                .execute(&pool)
                .await
                .unwrap();
            let ctx = Context {
                id,
                pool: pool.clone(),
                directory: root.path().join(format!("job-{id}")),
                stop: CancellationToken::new(),
            };
            std::fs::create_dir(&ctx.directory).unwrap();
            let artifact = Artifact::new(&destination).unwrap();
            std::fs::write(artifact.path().join("index.html"), timing).unwrap();
            if timing == "before" {
                assert!(cancel(&pool, id).await.unwrap());
                ctx.stop.cancel();
            }
            let result = artifact
                .publish(&ctx.stop)
                .map(|path| json!({"directory":path}));
            if timing != "before" {
                assert!(cancel(&pool, id).await.unwrap());
                if timing == "after-token" {
                    ctx.stop.cancel();
                }
            }
            finish(&ctx, result, false).await.unwrap();
            let job = list(&pool).await.unwrap()[0].clone();
            assert_eq!(job["state"], expected);
            assert_eq!(
                std::fs::read_to_string(destination.join("index.html")).unwrap(),
                if timing == "before" { "old" } else { timing }
            );
            assert!(!ctx.directory.exists());
            assert!(!cancel(&pool, id).await.unwrap());
            if expected == "succeeded" {
                assert_eq!(job["result"]["directory"], destination.to_str().unwrap());
                assert!(job["error"].is_null());
            }
        }
    }
}
