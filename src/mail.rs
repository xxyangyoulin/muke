//! 极简 SMTP 客户端（465 隐式 TLS）：自托管够用，失败只记日志不致命。
//! 配置（后台评论设置）：smtp_host / smtp_port / smtp_user / smtp_pass / notify_email

use crate::{api::App, config::Settings};
use anyhow::{Result, bail};
use base64::Engine;
use std::{sync::Arc, time::Duration};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};
use tokio_rustls::rustls::pki_types::ServerName;
use tokio_rustls::rustls::{ClientConfig, RootCertStore};
use tokio_util::sync::CancellationToken;

fn connector() -> &'static tokio_rustls::TlsConnector {
    static CONNECTOR: std::sync::LazyLock<tokio_rustls::TlsConnector> =
        std::sync::LazyLock::new(|| {
            let roots = RootCertStore::from_iter(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
            tokio_rustls::TlsConnector::from(Arc::new(
                ClientConfig::builder()
                    .with_root_certificates(roots)
                    .with_no_client_auth(),
            ))
        });
    &CONNECTOR
}

pub fn configured(settings: &Settings) -> bool {
    !settings.get("smtp_host").is_empty()
        && !settings.get("smtp_user").is_empty()
        && !settings.get("smtp_pass").is_empty()
}

async fn command<R, W>(
    reader: &mut BufReader<R>,
    writer: &mut W,
    line: &str,
    expected: &[u16],
    stop: &CancellationToken,
    timeout: Duration,
) -> Result<()>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    if !line.is_empty() {
        writer.write_all(format!("{line}\r\n").as_bytes()).await?;
    }
    loop {
        if stop.is_cancelled() {
            bail!("任务已取消");
        }
        let mut response = String::new();
        match tokio::time::timeout(timeout, reader.read_line(&mut response)).await {
            Ok(Ok(0)) => bail!("SMTP 连接已关闭"),
            Ok(Ok(_)) => {
                let code: u16 = response.get(..3).and_then(|s| s.parse().ok()).unwrap_or(0);
                if response.len() > 3 && response.as_bytes()[3] == b'-' {
                    continue;
                }
                if !expected.contains(&code) {
                    bail!("SMTP 命令 {line:?} 失败：{}", response.trim_end());
                }
                return Ok(());
            }
            Ok(Err(error)) => return Err(error.into()),
            Err(_) => bail!("SMTP 响应超时"),
        }
    }
}

/// 发送一封纯文本邮件；失败返回错误，由调用方记录日志，绝不打断业务。
pub async fn send(
    app: &App,
    to: &str,
    subject: &str,
    body: &str,
    stop: &CancellationToken,
) -> Result<()> {
    let settings = Settings::load(app).await?;
    if !configured(&settings) || to.is_empty() {
        return Ok(());
    }
    let host = settings.get("smtp_host").to_owned();
    let port = if settings.number("smtp_port") > 0 {
        settings.number("smtp_port") as u16
    } else {
        465
    };
    let user = settings.get("smtp_user").to_owned();
    let password = settings.get("smtp_pass").to_owned();
    let timeout = Duration::from_secs(15);
    let tcp = tokio::time::timeout(
        timeout,
        tokio::net::TcpStream::connect((host.as_str(), port)),
    )
    .await??;
    let tls = tokio::time::timeout(
        timeout,
        connector().connect(ServerName::try_from(host)?, tcp),
    )
    .await??;
    let (read, write) = tokio::io::split(tls);
    let mut reader = BufReader::new(read);
    let mut writer = write;
    command(&mut reader, &mut writer, "", &[220], stop, timeout).await?;
    command(&mut reader, &mut writer, "EHLO muke", &[250], stop, timeout).await?;
    command(
        &mut reader,
        &mut writer,
        "AUTH LOGIN",
        &[334],
        stop,
        timeout,
    )
    .await?;
    let engine = base64::engine::general_purpose::STANDARD;
    command(
        &mut reader,
        &mut writer,
        &engine.encode(&user),
        &[334],
        stop,
        timeout,
    )
    .await?;
    command(
        &mut reader,
        &mut writer,
        &engine.encode(&password),
        &[235],
        stop,
        timeout,
    )
    .await?;
    command(
        &mut reader,
        &mut writer,
        &format!("MAIL FROM:<{user}>"),
        &[250],
        stop,
        timeout,
    )
    .await?;
    command(
        &mut reader,
        &mut writer,
        &format!("RCPT TO:<{to}>"),
        &[250, 251],
        stop,
        timeout,
    )
    .await?;
    command(&mut reader, &mut writer, "DATA", &[354], stop, timeout).await?;
    let subject = format!("=?UTF-8?B?{}?=", engine.encode(subject));
    let from = format!("=?UTF-8?B?{}?=", engine.encode("木刻"));
    let data = format!(
        "From: {from} <{user}>\r\nTo: <{to}>\r\nSubject: {subject}\r\nMIME-Version: 1.0\r\nContent-Type: text/plain; charset=UTF-8\r\n\r\n{}",
        body.replace("\r\n", "\n").replace("\n.", "\n..")
    );
    command(
        &mut reader,
        &mut writer,
        &format!("{data}\r\n."),
        &[250],
        stop,
        timeout,
    )
    .await?;
    let _ = writer.write_all(b"QUIT\r\n").await;
    let _ = writer.shutdown().await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn unconfigured_mail_is_a_no_op() {
        assert!(!configured(&Settings(std::collections::BTreeMap::new())));
        let root = tempfile::tempdir().unwrap();
        let data = root.path().join("data");
        let app = App {
            db: crate::database::open(&data).await.unwrap(),
            tasks: crate::tasks::open(&root.path().join("work")).await.unwrap(),
            network: crate::network::Client::new(None),
            data,
            config: Arc::new(crate::config::Config::default()),
            shutdown: CancellationToken::new(),
            password_slots: Arc::new(tokio::sync::Semaphore::new(2)),
            media_slots: Arc::new(tokio::sync::Semaphore::new(2)),
            content_lock: Arc::new(tokio::sync::RwLock::new(())),
        };
        send(
            &app,
            "a@example.org",
            "标题",
            "内容",
            &CancellationToken::new(),
        )
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn command_rejects_unexpected_codes_and_multi_line_continues() {
        use tokio::io::duplex;
        let stop = CancellationToken::new();
        let timeout = Duration::from_secs(2);
        let (client, server) = duplex(4096);
        let (reader, mut writer) = tokio::io::split(client);
        let mut reader = BufReader::new(reader);
        let mut responder = server;
        let responder_task = tokio::spawn(async move {
            use tokio::io::AsyncWriteExt;
            responder
                .write_all(b"220-host multi\r\n220 ready\r\n")
                .await?;
            responder.write_all(b"500 nope\r\n").await?;
            responder.flush().await?;
            Ok::<_, std::io::Error>(())
        });
        command(&mut reader, &mut writer, "", &[220], &stop, timeout)
            .await
            .unwrap();
        assert!(
            command(
                &mut reader,
                &mut writer,
                "EHLO muke",
                &[250],
                &stop,
                timeout
            )
            .await
            .is_err()
        );
        responder_task.await.unwrap().unwrap();
    }
}
