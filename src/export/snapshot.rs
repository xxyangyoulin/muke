use crate::{
    api::{App, public_state, row_json},
    comments,
    config::Settings,
    plugins,
};
use anyhow::{Result, bail};
use serde_json::Value;
use std::collections::BTreeMap;
use tokio::sync::RwLockReadGuard;
use tokio_util::sync::CancellationToken;

pub struct Snapshot<'a> {
    pub state: Value,
    pub site_url: String,
    pub posts: Vec<Value>,
    pub articles: Vec<Value>,
    pub comments: BTreeMap<String, Vec<Value>>,
    pub hidden_sources: Vec<String>,
    pub plugins: BTreeMap<String, plugins::ExportSnapshot>,
    // Keep referenced media alive until the exporter has copied them into its staging directory.
    pub(super) _guard: RwLockReadGuard<'a, ()>,
}

pub async fn capture<'a>(app: &'a App, stop: &CancellationToken) -> Result<Snapshot<'a>> {
    let guard = tokio::select! {
        _ = stop.cancelled() => bail!("任务已取消"),
        guard = app.content_lock.read() => guard,
    };
    let settings = Settings::load(app).await?;
    let mut state = public_state(app, &settings).await?;
    state["capabilities"]["read_only"] = Value::Bool(true);
    let hidden_sources = plugins::hidden_sources(app).await?;
    let plugins = plugins::export_snapshots(app).await?;
    let posts = sqlx::query("SELECT p.id,p.content,p.created_at,p.updated_at,p.images,p.meta,p.source,p.source_key,p.link,p.pinned,(SELECT COUNT(*) FROM post_likes l WHERE l.post_id=p.id) AS like_count,(SELECT COUNT(*) FROM comments c WHERE c.kind='post' AND c.target_id=p.id AND c.status='approved') AS comment_count FROM posts p ORDER BY pinned DESC,created_at DESC,id DESC")
        .fetch_all(&app.db).await?.into_iter().map(row_json).collect();
    let articles = sqlx::query("SELECT a.id,a.title,a.content,a.format,a.images,a.tags,a.created_at,a.updated_at,(SELECT COUNT(*) FROM comments c WHERE c.kind='article' AND c.target_id=a.id AND c.status='approved') AS comment_count FROM articles a WHERE draft=0 ORDER BY created_at DESC,id DESC")
        .fetch_all(&app.db).await?.into_iter().map(row_json).collect();
    let mut comments = BTreeMap::<String, Vec<Value>>::new();
    if settings.get("comment_provider") != "off" {
        let rows = sqlx::query("SELECT c.id,c.kind,c.target_id,c.author,c.email,c.site,c.content,c.created_at,c.reply_to,(SELECT author FROM comments p WHERE p.id=c.reply_to AND p.kind=c.kind AND p.target_id=c.target_id AND p.status='approved') AS reply_name FROM comments c WHERE c.status='approved' AND ((c.kind='article' AND EXISTS(SELECT 1 FROM articles a WHERE a.id=c.target_id AND a.draft=0)) OR (c.kind='post' AND ?<>'article' AND EXISTS(SELECT 1 FROM posts p WHERE p.id=c.target_id))) ORDER BY c.id")
            .bind(settings.get("comment_scope")).fetch_all(&app.db).await?;
        for row in rows {
            let mut comment = comments::public_comment(row);
            let object = comment.as_object_mut().unwrap();
            let kind = object.remove("kind").unwrap();
            let target = object.remove("target_id").unwrap();
            comments
                .entry(format!(
                    "{}:{}",
                    kind.as_str().unwrap(),
                    target.as_i64().unwrap()
                ))
                .or_default()
                .push(comment);
        }
    }
    if stop.is_cancelled() {
        bail!("任务已取消");
    }
    Ok(Snapshot {
        state,
        site_url: settings.get("site_url").into(),
        posts,
        articles,
        comments,
        hidden_sources,
        plugins,
        _guard: guard,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{config::Config, database, network, tasks};
    use serde_json::json;
    use std::sync::Arc;

    #[tokio::test]
    async fn public_snapshot_excludes_drafts_credentials_and_private_comments() {
        let root = tempfile::tempdir().unwrap();
        let data = root.path().join("data");
        let app = App {
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
        for key in [
            "netlify_token",
            "smtp_password",
            "akismet_key",
            "giscus_token",
        ] {
            sqlx::query("INSERT INTO settings VALUES(?,?)")
                .bind(key)
                .bind(format!("secret-{key}"))
                .execute(&app.db)
                .await
                .unwrap();
        }
        for (key, value) in [
            ("plugin:rss:enabled", "1"),
            ("plugin:rss:in_feed", "0"),
            ("comment_provider", "native"),
        ] {
            sqlx::query("INSERT INTO settings VALUES(?,?)")
                .bind(key)
                .bind(value)
                .execute(&app.db)
                .await
                .unwrap();
        }
        sqlx::query("INSERT INTO users VALUES(1,'private-account','secret-password')")
            .execute(&app.db)
            .await
            .unwrap();
        sqlx::query("INSERT INTO posts(content,source,created_at,updated_at) VALUES('post','','2026','2026'),('feed','rss','2026','2026')").execute(&app.db).await.unwrap();
        sqlx::query("INSERT INTO post_likes VALUES(1,'secret-visitor-token','2026')")
            .execute(&app.db)
            .await
            .unwrap();
        sqlx::query("INSERT INTO articles(title,content,draft,created_at,updated_at) VALUES('public','body',0,'2026','2026'),('secret-draft','secret-body',1,'2026','2026')").execute(&app.db).await.unwrap();
        for (kind, target, status, content) in [
            ("post", 1, "approved", "public-post-comment"),
            ("article", 1, "approved", "public-article-comment"),
            ("article", 2, "approved", "secret-draft-comment"),
            ("post", 1, "pending", "secret-pending-comment"),
            ("post", 1, "spam", "secret-spam-comment"),
            ("post", 999, "approved", "secret-orphan-comment"),
        ] {
            sqlx::query("INSERT INTO comments(kind,target_id,author,email,content,created_at,status,ip_hash) VALUES(?,?,'author','private@example.org',?,'2026',?,'secret-ip')")
                .bind(kind).bind(target).bind(content).bind(status).execute(&app.db).await.unwrap();
        }
        let stop = CancellationToken::new();
        let snapshot = capture(&app, &stop).await.unwrap();
        assert_eq!(snapshot.articles.len(), 1);
        assert_eq!(snapshot.posts.len(), 2);
        assert_eq!(snapshot.hidden_sources, vec!["rss"]);
        assert_eq!(snapshot.comments.len(), 2);
        assert_eq!(snapshot.comments["post:1"].len(), 1);
        assert!(
            !snapshot.comments["post:1"][0]["email_hash"]
                .as_str()
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            snapshot.posts.iter().find(|post| post["id"] == 1).unwrap()["like_count"],
            1
        );
        assert_eq!(snapshot.state["admin"], false);
        assert_eq!(snapshot.state["setup"], false);
        assert_eq!(snapshot.state["capabilities"]["read_only"], true);
        assert!(snapshot.state.get("csrf").is_none());
        let output = json!({"state":snapshot.state,"posts":snapshot.posts,"articles":snapshot.articles,"comments":snapshot.comments}).to_string();
        for private in [
            "secret-",
            "private-account",
            "private@example.org",
            "ip_hash",
            "netlify",
            "smtp_password",
            "akismet_key",
            "giscus_token",
        ] {
            assert!(!output.contains(private), "leaked {private}");
        }
        assert!(app.content_lock.try_write().is_err());
        drop(snapshot);
        assert!(app.content_lock.try_write().is_ok());
        sqlx::query("INSERT INTO settings VALUES('comment_scope','article')")
            .execute(&app.db)
            .await
            .unwrap();
        let snapshot = capture(&app, &stop).await.unwrap();
        assert_eq!(snapshot.comments.len(), 1);
        assert!(snapshot.comments.contains_key("article:1"));
        drop(snapshot);
        sqlx::query("UPDATE settings SET value='off' WHERE key='comment_provider'")
            .execute(&app.db)
            .await
            .unwrap();
        assert!(capture(&app, &stop).await.unwrap().comments.is_empty());
        let _writer = app.content_lock.write().await;
        stop.cancel();
        assert!(capture(&app, &stop).await.is_err());
    }
}
