use super::{
    artifact::Artifact,
    assets,
    data::{self, Data},
    feeds, pages, snapshot,
};
use crate::{api::App, tasks};
use anyhow::{Result, bail};
use serde_json::{Value, json};
use std::path::Path;

/// 生成完整静态站点并原子发布到 root/dist：资源、数据、页面、RSS、sitemap 与辅助文件。
/// 取消或任一阶段失败时保留旧版产物；返回导出摘要。
pub async fn export(
    app: &App,
    frontend: &Path,
    root: &Path,
    ctx: &tasks::Context,
) -> Result<Value> {
    let artifact = Artifact::new(root)?;
    let destination = artifact.path().to_owned();
    ctx.progress("读取公开数据", 0, None).await?;
    let snapshot = snapshot::capture(app, &ctx.stop).await?;
    let enabled: Vec<String> = snapshot.state["plugins"]
        .as_object()
        .map(|plugins| plugins.keys().cloned().collect())
        .unwrap_or_default();
    ctx.progress("复制资源", 0, None).await?;
    assets::copy(frontend, &destination, &enabled, &ctx.stop).await?;
    let data = data::build(app, snapshot, &destination, &ctx.stop).await?;
    ctx.progress("生成页面", 0, None).await?;
    pages::write(frontend, &destination, &data, &ctx.stop)?;
    ctx.progress("生成订阅与辅助文件", 0, None).await?;
    feeds::write(&destination, &data, &ctx.stop)?;
    write_extras(&destination, &data)?;
    for file in [
        "index.html",
        "assets/content.json",
        "assets/articles.json",
        "rss.xml",
        "sitemap.xml",
    ] {
        if !destination.join(file).is_file() {
            bail!("导出产物不完整：{file}");
        }
    }
    ctx.progress("发布", 0, None).await?;
    let dist = artifact.publish(&ctx.stop)?;
    let count = |value: &Value| {
        value
            .as_array()
            .map(|items| items.len())
            .unwrap_or_default()
    };
    let mut plugins = serde_json::Map::new();
    if let Some(items) = data.content["plugins"].as_object() {
        for (id, item) in items {
            let count = item
                .get("posts")
                .and_then(Value::as_array)
                .or_else(|| item.as_array())
                .map(|items| items.len())
                .unwrap_or_default();
            plugins.insert(id.clone(), json!(count));
        }
    }
    Ok(json!({
        "ok": true,
        "posts": count(&data.content["posts"]),
        "articles": count(&data.articles["articles"]),
        "plugins": plugins,
        "directory": dist.to_string_lossy(),
    }))
}

/// 404 页、_headers 与同源评论嵌入页（waline / twikoo）。
fn write_extras(destination: &Path, data: &Data) -> Result<()> {
    let options = &data.content["state"]["options"];
    let provider = options["comment_provider"].as_str().unwrap_or("");
    std::fs::write(
        destination.join("404.html"),
        r#"<!doctype html><html lang="zh-CN"><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>页面不存在</title><link rel="stylesheet" href="/assets/style.css"><main class="empty-state"><h1>这片风景还未抵达</h1><p>你访问的页面不存在。</p><a href="/">回到首页</a></main></html>"#,
    )?;
    let mut extra = String::new();
    if provider == "giscus" {
        extra = "; frame-src 'self' https://giscus.app".into();
    } else if provider == "custom"
        && let Some(origin) = url::Url::parse(options["comment_embed_url"].as_str().unwrap_or(""))
            .ok()
            .filter(|url| matches!(url.scheme(), "http" | "https"))
    {
        extra = format!(
            "; frame-src 'self' {}",
            origin.origin().ascii_serialization()
        );
    }
    let mut headers = format!(
        "/*\n  X-Content-Type-Options: nosniff\n  X-Frame-Options: DENY\n  Referrer-Policy: strict-origin-when-cross-origin\n  Content-Security-Policy: default-src 'self'; img-src 'self' data: blob:; style-src 'self'; script-src 'self'; base-uri 'self'; frame-ancestors 'none'; form-action 'self'{extra}\n"
    );
    if matches!(provider, "waline" | "twikoo") {
        let provider = pages::escape(provider);
        let address = pages::escape(options["comment_embed_url"].as_str().unwrap_or(""));
        let embed = destination.join("embed").join("comments");
        std::fs::create_dir_all(&embed)?;
        std::fs::write(
            embed.join("index.html"),
            format!(
                r#"<!doctype html><html lang="zh-CN"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1"><link rel="stylesheet" href="/assets/embed.css"><title>评论</title></head><body><div id="embed-root"></div><script src="/assets/embed-init.js" data-provider="{provider}" data-url="{address}" defer></script></body></html>"#
            ),
        )?;
        headers.push_str("/embed/*\n  Content-Security-Policy: default-src 'self'; script-src 'self' https://unpkg.com https://cdn.jsdelivr.net; style-src 'self' 'unsafe-inline' https://unpkg.com https://cdn.jsdelivr.net; img-src * data: blob:; font-src * data:; connect-src *; frame-ancestors 'self'\n");
    }
    std::fs::write(destination.join("_headers"), headers)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{config::Config, database, export::data::Data, network, plugins, tasks};
    use serde_json::json;
    use std::sync::Arc;
    use tokio_util::sync::CancellationToken;

    async fn app(root: &Path) -> App {
        let data = root.join("data");
        let app = App {
            db: database::open(&data).await.unwrap(),
            tasks: tasks::open(&root.join("work")).await.unwrap(),
            network: network::Client::new(None),
            data,
            config: Arc::new(Config::default()),
            shutdown: CancellationToken::new(),
            password_slots: Arc::new(tokio::sync::Semaphore::new(2)),
            media_slots: Arc::new(tokio::sync::Semaphore::new(2)),
            content_lock: Arc::new(tokio::sync::RwLock::new(())),
        };
        plugins::migrate(&app).await.unwrap();
        sqlx::query("INSERT INTO posts(content,source,images,meta,created_at,updated_at) VALUES('动态正文','','[]','{}','2026-11-26T08:00:00+08:00','2026-11-26T08:00:00+08:00')")
            .execute(&app.db)
            .await
            .unwrap();
        sqlx::query("INSERT INTO articles(title,content,format,created_at,updated_at) VALUES('文章标题','正文','html','2026-11-26T08:00:00+08:00','2026-11-26T08:00:00+08:00')")
            .execute(&app.db)
            .await
            .unwrap();
        app
    }

    #[tokio::test]
    async fn full_export_publishes_dist_and_keeps_old_version_on_cancel() {
        let root = tempfile::tempdir().unwrap();
        let app = app(root.path()).await;
        sqlx::query("INSERT INTO settings VALUES('comment_provider','waline')")
            .execute(&app.db)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO settings VALUES('comment_embed_url','https://comments.example.org')",
        )
        .execute(&app.db)
        .await
        .unwrap();
        let frontend = Path::new(env!("CARGO_MANIFEST_DIR")).join("frontend");
        sqlx::query("INSERT INTO jobs(id,kind,resource,dedup,payload,state) VALUES(1,'export','export','fixture','{}','running')")
            .execute(&app.tasks)
            .await
            .unwrap();
        let ctx = tasks::Context {
            id: 1,
            pool: app.tasks.clone(),
            directory: root.path().join("job-1"),
            stop: CancellationToken::new(),
        };
        let result = export(&app, &frontend, &root.path().join("site"), &ctx)
            .await
            .unwrap();
        let dist = Path::new(result["directory"].as_str().unwrap()).to_owned();
        assert_eq!(result["posts"], 1);
        assert_eq!(result["articles"], 1);
        assert_eq!(result["plugins"].as_object().unwrap().len(), 0);
        for file in [
            "index.html",
            "articles/index.html",
            "article/1/index.html",
            "assets/content.json",
            "assets/articles.json",
            "assets/style.css",
            "rss.xml",
            "sitemap.xml",
            "robots.txt",
            "404.html",
            "_headers",
            "embed/comments/index.html",
        ] {
            assert!(dist.join(file).is_file(), "缺少 {file}");
        }
        assert!(!dist.join(".media-sources").exists());
        let headers = std::fs::read_to_string(dist.join("_headers")).unwrap();
        assert!(headers.contains("form-action 'self'\n"));
        assert!(!headers.contains("frame-src"));
        assert!(headers.contains("/embed/*\n"));
        let embed = std::fs::read_to_string(dist.join("embed/comments/index.html")).unwrap();
        assert!(
            embed.contains(r#"data-provider="waline" data-url="https://comments.example.org""#)
        );
        assert_eq!(
            std::fs::read_to_string(dist.join("robots.txt")).unwrap(),
            "User-agent: *\nAllow: /\n\nSitemap: /sitemap.xml\n"
        );
        let phase: String = sqlx::query_scalar("SELECT phase FROM jobs WHERE id=1")
            .fetch_one(&app.tasks)
            .await
            .unwrap();
        assert_eq!(phase, "发布");
        // 再次导出替换旧版，取消时保留旧版
        sqlx::query("INSERT INTO articles(title,content,format,created_at,updated_at) VALUES('第二篇','正文','html','2026-11-27T08:00:00+08:00','2026-11-27T08:00:00+08:00')")
            .execute(&app.db)
            .await
            .unwrap();
        let result = export(&app, &frontend, &root.path().join("site"), &ctx)
            .await
            .unwrap();
        assert_eq!(result["articles"], 2);
        assert!(dist.join("article/2/index.html").is_file());
        let cancelled = CancellationToken::new();
        cancelled.cancel();
        let ctx = tasks::Context {
            id: 1,
            pool: app.tasks.clone(),
            directory: root.path().join("job-1"),
            stop: cancelled,
        };
        assert!(
            export(&app, &frontend, &root.path().join("site"), &ctx)
                .await
                .is_err()
        );
        assert!(dist.join("article/2/index.html").is_file());
        assert_eq!(
            std::fs::read_dir(root.path())
                .unwrap()
                .filter_map(std::result::Result::ok)
                .filter(|entry| entry.file_name().to_string_lossy().starts_with(".export-"))
                .count(),
            0
        );
    }

    #[test]
    fn headers_follow_comment_provider() {
        let mut data = Data {
            site_url: String::new(),
            content: json!({"state":{"options":{"comment_provider":"giscus"}}}),
            articles: json!({"articles":[]}),
        };
        let root = tempfile::tempdir().unwrap();
        write_extras(root.path(), &data).unwrap();
        let headers = std::fs::read_to_string(root.path().join("_headers")).unwrap();
        assert!(headers.contains("; frame-src 'self' https://giscus.app\n"));
        assert!(!headers.contains("/embed/*"));
        assert!(!root.path().join("embed").exists());
        data.content["state"]["options"]["comment_provider"] = json!("custom");
        data.content["state"]["options"]["comment_embed_url"] =
            json!("https://talk.example.org:8443/path");
        write_extras(root.path(), &data).unwrap();
        let headers = std::fs::read_to_string(root.path().join("_headers")).unwrap();
        assert!(headers.contains("; frame-src 'self' https://talk.example.org:8443\n"));
        data.content["state"]["options"]["comment_embed_url"] = json!("javascript:alert(1)");
        write_extras(root.path(), &data).unwrap();
        let headers = std::fs::read_to_string(root.path().join("_headers")).unwrap();
        assert!(headers.contains("form-action 'self'\n"));
        data.content["state"]["options"] = json!({"comment_provider":"off"});
        write_extras(root.path(), &data).unwrap();
        let headers = std::fs::read_to_string(root.path().join("_headers")).unwrap();
        assert!(!headers.contains("frame-src"));
        let page = std::fs::read_to_string(root.path().join("404.html")).unwrap();
        assert!(page.contains("这片风景还未抵达"));
    }
}
