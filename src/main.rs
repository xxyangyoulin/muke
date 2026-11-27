use anyhow::{Context, Result, bail};
use axum::{Router, routing::get};
use clap::{Parser, Subcommand};
use muke::{
    api::{App, api, image_file},
    tasks,
};
use muke::{auth, config::Config, database, mutations};
use std::{net::SocketAddr, path::PathBuf, sync::Arc};
use tokio_util::sync::CancellationToken;
use tower_http::{services::ServeDir, set_header::SetResponseHeaderLayer};

#[derive(Parser)]
#[command(about = "MUKE：内容管理与后台任务服务")]
struct Args {
    #[arg(long)]
    data_dir: Option<PathBuf>,
    #[arg(long)]
    config: Option<PathBuf>,
    #[arg(long)]
    work_dir: Option<PathBuf>,
    #[command(subcommand)]
    command: Action,
}
#[derive(Subcommand)]
enum Action {
    Serve {
        #[arg(long)]
        listen: Option<SocketAddr>,
        #[arg(long)]
        frontend: Option<PathBuf>,
    },
    Thumbnail {
        input: PathBuf,
    },
    Tasks,
    Cancel {
        id: i64,
    },
    Export,
    Backup {
        output: PathBuf,
    },
    Restore {
        archive: PathBuf,
        target: PathBuf,
    },
    Check,
}
#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "muke=info".into()),
        )
        .init();
    let args = Args::parse();
    let mut config = Config::load(args.config.as_deref())?;
    if let Some(data) = args.data_dir {
        config.data_dir = data;
    }
    std::fs::create_dir_all(&config.data_dir)?;
    let data = std::fs::canonicalize(&config.data_dir)?;
    let work = args.work_dir.unwrap_or_else(|| data.join("runtime"));
    std::fs::create_dir_all(&work)?;
    let work = std::fs::canonicalize(work)?;
    if data.starts_with(&work) {
        bail!("任务目录不能包含内容目录");
    }
    match args.command {
        Action::Serve { listen, frontend } => {
            let listen = listen.unwrap_or(config.listen);
            let frontend = frontend.unwrap_or_else(|| config.frontend.clone());
            if !frontend.join("index.html").is_file() {
                bail!("前端目录缺少 index.html");
            }
            config.frontend = frontend.clone();
            let lock = std::fs::OpenOptions::new()
                .create(true)
                .truncate(false)
                .write(true)
                .open(data.join("service.lock"))?;
            fs2::FileExt::try_lock_exclusive(&lock).context("该任务目录已有服务运行")?;
            let pool = tasks::open(&work).await?;
            tasks::recover(&pool, &work).await?;
            muke::export::artifact::recover(&config.export_dir)?;
            let db = database::open(&data).await?;
            let stop = CancellationToken::new();
            let state = App {
                tasks: pool,
                network: muke::network::Client::new(config.http_proxy.clone()),
                shutdown: stop.clone(),
                db,
                data,
                config: Arc::new(config),
                password_slots: Arc::new(tokio::sync::Semaphore::new(2)),
                media_slots: Arc::new(tokio::sync::Semaphore::new(2)),
                content_lock: Arc::new(tokio::sync::RwLock::new(())),
            };
            muke::media::recover(&state).await?;
            muke::plugins::migrate(&state).await?;
            let worker_state = state.clone();
            let plugin_assets = frontend
                .parent()
                .unwrap_or(std::path::Path::new("."))
                .join("plugins");
            let app = Router::new()
                .route("/api", get(api).post(mutations::post))
                .route("/api/tasks/events", get(tasks::events))
                .route_layer(axum::middleware::from_fn_with_state(
                    state.clone(),
                    auth::middleware,
                ))
                .route("/image", get(image_file));
            let app=muke::plugins::assets(app, &plugin_assets)
                .route("/health",get(||async {axum::Json(serde_json::json!({"ok":true}))}))
                .fallback_service(ServeDir::new(&frontend).fallback(tower_http::services::ServeFile::new(frontend.join("index.html"))))
                .with_state(state)
                .layer(axum::extract::DefaultBodyLimit::max(68*1024*1024))
                .layer(SetResponseHeaderLayer::overriding(axum::http::header::CACHE_CONTROL,axum::http::HeaderValue::from_static("no-store")))
                .layer(SetResponseHeaderLayer::overriding(axum::http::header::X_CONTENT_TYPE_OPTIONS,axum::http::HeaderValue::from_static("nosniff")))
                .layer(SetResponseHeaderLayer::overriding(axum::http::header::CONTENT_SECURITY_POLICY,axum::http::HeaderValue::from_static("default-src 'self'; img-src 'self' https: data: blob:; script-src 'self'; style-src 'self'; frame-src 'self' https://giscus.app; frame-ancestors 'none'; base-uri 'self'")));
            let listener = tokio::net::TcpListener::bind(listen).await?;
            println!("MUKE http://{}", listener.local_addr()?);
            let signal_stop = stop.clone();
            tokio::spawn(async move {
                let mut term =
                    tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                        .expect("SIGTERM handler");
                tokio::select! { _=tokio::signal::ctrl_c()=>{}, _=term.recv()=>{} }
                signal_stop.cancel();
            });
            let worker_stop = stop.clone();
            let worker = tokio::spawn(async move {
                let result = tasks::run(worker_state, work, worker_stop.clone()).await;
                worker_stop.cancel();
                result
            });
            let server = axum::serve(
                listener,
                app.into_make_service_with_connect_info::<SocketAddr>(),
            )
            .with_graceful_shutdown(stop.clone().cancelled_owned())
            .into_future();
            use std::future::IntoFuture;
            tokio::pin!(server);
            let result = tokio::select! {
                result=&mut server => result,
                _=stop.cancelled()=>tokio::time::timeout(std::time::Duration::from_secs(35),&mut server).await.context("HTTP 关闭超时")?,
            };
            stop.cancel();
            tokio::time::timeout(std::time::Duration::from_secs(10), worker)
                .await
                .context("任务关闭超时")???;
            result?;
            drop(lock);
        }
        action => {
            let pool = tasks::open(&work).await?;
            match action {
                Action::Thumbnail { input } => {
                    println!("{}", tasks::enqueue(&pool, &data, &input).await?)
                }
                Action::Tasks => println!(
                    "{}",
                    serde_json::to_string_pretty(&tasks::list(&pool).await?)?
                ),
                Action::Cancel { id } => println!("{}", tasks::cancel(&pool, id).await?),
                Action::Export => {
                    let frontend = config.frontend.clone();
                    if !frontend.join("index.html").is_file() {
                        bail!("前端目录缺少 index.html");
                    }
                    tasks::recover(&pool, &work).await?;
                    let db = database::open(&data).await?;
                    let stop = CancellationToken::new();
                    let state = App {
                        tasks: pool.clone(),
                        network: muke::network::Client::new(config.http_proxy.clone()),
                        shutdown: stop.clone(),
                        db,
                        data,
                        config: Arc::new(config),
                        password_slots: Arc::new(tokio::sync::Semaphore::new(2)),
                        media_slots: Arc::new(tokio::sync::Semaphore::new(2)),
                        content_lock: Arc::new(tokio::sync::RwLock::new(())),
                    };
                    muke::plugins::migrate(&state).await?;
                    let worker_state = state.clone();
                    let worker_stop = stop.clone();
                    let worker = tokio::spawn(async move {
                        let result = tasks::run(worker_state, work, worker_stop.clone()).await;
                        worker_stop.cancel();
                        result
                    });
                    let id = tasks::submit(&state.tasks, &tasks::Task::Export {}).await?;
                    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(600);
                    let job = loop {
                        let job = tasks::list(&state.tasks)
                            .await?
                            .as_array()
                            .unwrap()
                            .iter()
                            .find(|job| job["id"].as_i64() == Some(id))
                            .cloned()
                            .context("任务记录丢失")?;
                        if !matches!(job["state"].as_str(), Some("queued" | "running")) {
                            break job;
                        }
                        if std::time::Instant::now() > deadline {
                            tasks::cancel(&state.tasks, id).await?;
                            bail!("导出任务超时");
                        }
                        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                    };
                    stop.cancel();
                    tokio::time::timeout(std::time::Duration::from_secs(10), worker)
                        .await
                        .context("任务关闭超时")???;
                    if let Some(error) = job["error"].as_str() {
                        bail!("导出失败：{error}");
                    }
                    println!("{}", serde_json::to_string_pretty(&job["result"])?);
                }
                Action::Backup { output } => {
                    let db = database::open(&data).await?;
                    let stop = CancellationToken::new();
                    let state = App {
                        tasks: pool,
                        network: muke::network::Client::new(config.http_proxy.clone()),
                        shutdown: stop.clone(),
                        db,
                        data,
                        config: Arc::new(config),
                        password_slots: Arc::new(tokio::sync::Semaphore::new(2)),
                        media_slots: Arc::new(tokio::sync::Semaphore::new(2)),
                        content_lock: Arc::new(tokio::sync::RwLock::new(())),
                    };
                    muke::plugins::migrate(&state).await?;
                    let output = if output.is_absolute() {
                        output
                    } else {
                        std::env::current_dir()?.join(output)
                    };
                    muke::backup::create(&state, &output, &stop).await?;
                    println!("{}", output.display());
                }
                Action::Restore { archive, target } => {
                    muke::backup::restore(&archive, &target).await?;
                    println!("{}", std::fs::canonicalize(target)?.display());
                }
                Action::Check => {
                    let mut fail = 0;
                    println!("MUKE {}", env!("CARGO_PKG_VERSION"));
                    let mut check = |name: &str, ok: bool, hint: &str| {
                        println!(
                            "{} {}{}",
                            if ok { "[ OK ]" } else { "[FAIL]" },
                            name,
                            if ok || hint.is_empty() {
                                String::new()
                            } else {
                                format!(" —— {hint}")
                            }
                        );
                        if !ok {
                            fail += 1;
                        }
                    };
                    let magick = std::process::Command::new("magick")
                        .arg("-version")
                        .stdout(std::process::Stdio::null())
                        .stderr(std::process::Stdio::null())
                        .status()
                        .map(|status| status.success())
                        .unwrap_or(false);
                    check(
                        "ImageMagick (magick)",
                        magick,
                        "缩略图需要；Arch: pacman -S imagemagick；Debian: apt install imagemagick",
                    );
                    let writable = |path: &std::path::Path| {
                        std::fs::create_dir_all(path).is_ok()
                            && std::fs::OpenOptions::new()
                                .append(true)
                                .create(true)
                                .open(path.join(".write-test"))
                                .and_then(|file| {
                                    drop(file);
                                    std::fs::remove_file(path.join(".write-test"))
                                })
                                .is_ok()
                    };
                    check(
                        "数据目录可写",
                        writable(&config.data_dir),
                        &config.data_dir.display().to_string(),
                    );
                    check(
                        "前端目录包含 index.html",
                        config.frontend.join("index.html").is_file(),
                        &config.frontend.display().to_string(),
                    );
                    let plugins = config
                        .frontend
                        .parent()
                        .unwrap_or(std::path::Path::new("."))
                        .join("plugins");
                    let missing: Vec<_> = muke::plugins::frontend_directories(&plugins)
                        .into_iter()
                        .filter(|(_, directory)| !directory.is_dir())
                        .map(|(id, _)| id.to_owned())
                        .collect();
                    check("插件前端目录齐备", missing.is_empty(), &missing.join("、"));
                    if fail == 0 {
                        println!("\n环境就绪 ✓");
                    } else {
                        println!("\n有 {fail} 项未通过");
                        bail!("自检未通过");
                    }
                }
                Action::Serve { .. } => unreachable!(),
            }
        }
    }
    Ok(())
}
