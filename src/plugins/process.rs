use anyhow::{Context, Result, bail};
use std::{process::Stdio, time::Duration};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    process::{Child, Command},
};
use tokio_util::sync::CancellationToken;

pub(super) struct Process {
    pub child: Child,
    group: i32,
}

impl Process {
    pub fn spawn(command: &mut Command) -> Result<Self> {
        command
            .process_group(0)
            .kill_on_drop(true)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let child = command
            .spawn()
            .context("无法启动插件命令，请检查运行环境")?;
        let group = child.id().unwrap() as i32;
        Ok(Self { child, group })
    }

    pub async fn stop(&mut self) {
        // Cargo can have rustc/build-script children; cancellation owns the entire group.
        unsafe {
            libc::kill(-self.group, libc::SIGKILL);
        }
        let _ = self.child.wait().await;
    }
}

impl Drop for Process {
    fn drop(&mut self) {
        unsafe {
            libc::kill(-self.group, libc::SIGKILL);
        }
    }
}

pub(super) async fn line<R: tokio::io::AsyncBufRead + Unpin>(
    reader: &mut R,
) -> Result<Option<Vec<u8>>> {
    let mut bytes = Vec::new();
    let size = reader
        .take(8 * 1024 * 1024 + 1)
        .read_until(b'\n', &mut bytes)
        .await?;
    if size > 8 * 1024 * 1024 {
        bail!("插件输出单行超过 8 MB");
    }
    Ok((size != 0).then_some(bytes))
}

async fn logs<R: tokio::io::AsyncRead + Unpin>(
    reader: R,
    ctx: Option<&crate::tasks::Context>,
) -> Result<()> {
    let mut reader = BufReader::new(reader);
    while let Some(bytes) = line(&mut reader).await? {
        if let Some(ctx) = ctx {
            ctx.log(&String::from_utf8_lossy(&bytes)).await?;
        }
    }
    Ok(())
}

pub(super) async fn build(command: &mut Command, ctx: &crate::tasks::Context) -> Result<()> {
    ctx.check()?;
    let mut process = Process::spawn(command)?;
    drop(process.child.stdin.take());
    let stdout = process.child.stdout.take().unwrap();
    let stderr = process.child.stderr.take().unwrap();
    let work = async {
        tokio::try_join!(logs(stdout, Some(ctx)), logs(stderr, Some(ctx)))?;
        if !process.child.wait().await?.success() {
            bail!("插件命令执行失败，详情见编译日志");
        }
        Ok(())
    };
    let result = tokio::select! {
        _ = ctx.stop.cancelled() => Err(anyhow::anyhow!("任务已取消")),
        result = tokio::time::timeout(Duration::from_secs(1800), work) => result.context("插件编译超时")?,
    };
    process.stop().await;
    result
}

pub(super) async fn request(
    binary: &std::path::Path,
    input: &serde_json::Value,
    stop: &CancellationToken,
    ctx: Option<&crate::tasks::Context>,
    host: Option<&crate::api::App>,
) -> Result<serde_json::Value> {
    if stop.is_cancelled() {
        bail!("任务已取消");
    }
    let mut command = Command::new(binary);
    command.current_dir(binary.parent().unwrap()).env_clear();
    for name in ["PATH", "LANG", "TZ"] {
        if let Some(value) = std::env::var_os(name) {
            command.env(name, value);
        }
    }
    let mut process = Process::spawn(&mut command)?;
    let mut stdin = process.child.stdin.take().unwrap();
    let mut stdout = BufReader::new(process.child.stdout.take().unwrap());
    let stderr = process.child.stderr.take().unwrap();
    let work = async {
        stdin.write_all(&serde_json::to_vec(input)?).await?;
        stdin.write_all(b"\n").await?;
        stdin.flush().await?;
        let read = async {
            let mut result = None;
            while let Some(bytes) = line(&mut stdout).await? {
                let event: serde_json::Value =
                    serde_json::from_slice(&bytes).context("插件输出不是有效的 JSON 协议消息")?;
                match event["type"].as_str() {
                    Some("host_call") if result.is_none() => {
                        let id = event["id"]
                            .as_str()
                            .filter(|id| !id.is_empty() && id.len() <= 100)
                            .context("主程序调用编号无效")?;
                        let reply = async {
                            let app = host.context("此阶段不能调用主程序能力")?;
                            let ctx = ctx.context("模型调用必须在后台任务中执行")?;
                            if event["method"] != "llm.generate" {
                                bail!("主程序能力不存在");
                            }
                            let plugin = input["plugin"].as_str().context("插件标识缺失")?;
                            if !input["capabilities"].as_array().is_some_and(|items| {
                                items.iter().any(|item| item == "llm.generate")
                            }) {
                                bail!("插件未声明 llm.generate 能力");
                            }
                            crate::llm::generate(app, ctx, plugin, &event["params"]).await
                        }
                        .await;
                        let reply = match reply {
                            Ok(value) => {
                                serde_json::json!({"type":"host_result","id":id,"result":value})
                            }
                            Err(error) => {
                                serde_json::json!({"type":"host_result","id":id,"error":error.to_string()})
                            }
                        };
                        stdin.write_all(&serde_json::to_vec(&reply)?).await?;
                        stdin.write_all(b"\n").await?;
                        stdin.flush().await?;
                    }
                    Some("progress") if result.is_none() => {
                        if let Some(ctx) = ctx {
                            let phase = event["phase"]
                                .as_str()
                                .filter(|phase| phase.len() <= 1000)
                                .context("插件进度文字无效")?;
                            ctx.progress(
                                phase,
                                event["completed"].as_i64().unwrap_or(0),
                                event["total"].as_i64(),
                            )
                            .await?;
                        }
                    }
                    Some("result") if result.is_none() => {
                        result = Some(event);
                    }
                    Some("error") => bail!(
                        "插件执行失败：{}",
                        event["message"].as_str().unwrap_or("未知错误")
                    ),
                    _ => bail!("插件协议消息类型或顺序无效"),
                }
            }
            result.context("插件未返回结果")
        };
        let (value, _) = tokio::try_join!(read, logs(stderr, ctx))?;
        if !process.child.wait().await?.success() {
            bail!("插件进程异常退出");
        }
        Ok(value)
    };
    let result = tokio::select! {
        _ = stop.cancelled() => Err(anyhow::anyhow!("任务已取消")),
        result = tokio::time::timeout(Duration::from_secs(if ctx.is_some() { 600 } else { 30 }), work) => result.unwrap_or_else(|_| Err(anyhow::anyhow!("插件执行超时"))),
    };
    process.stop().await;
    if let (Some(app), Some(ctx)) = (host, ctx) {
        let _guard = app.content_lock.write().await;
        sqlx::query("UPDATE llm_calls SET status=? WHERE task_id=? AND status='running'")
            .bind(if stop.is_cancelled() {
                "cancelled"
            } else {
                "failed"
            })
            .bind(ctx.id)
            .execute(&app.db)
            .await?;
    }
    result
}
