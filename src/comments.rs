use crate::{
    api::{App, row_json},
    auth,
    config::Settings,
    error::{Error, Result},
};
use md5::{Digest, Md5};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    net::IpAddr,
};
use tokio_util::sync::CancellationToken;

pub async fn list(app: &App, kind: &str, target: i64) -> Result<Value> {
    if !matches!(kind, "post" | "article") {
        return Ok(json!({"comments":[]}));
    }
    let rows=sqlx::query("SELECT c.id,c.author,c.email,c.site,c.content,c.created_at,c.reply_to,(SELECT author FROM comments p WHERE p.id=c.reply_to AND p.kind=c.kind AND p.target_id=c.target_id AND p.status='approved') AS reply_name FROM comments c WHERE c.kind=? AND c.target_id=? AND c.status='approved' ORDER BY c.id")
        .bind(kind).bind(target).fetch_all(&app.db).await?;
    let comments: Vec<_> = rows.into_iter().map(public_comment).collect();
    Ok(json!({"comments":comments}))
}
pub(crate) fn public_comment(row: sqlx::sqlite::SqliteRow) -> Value {
    let mut value = row_json(row);
    let email = value
        .as_object_mut()
        .unwrap()
        .remove("email")
        .unwrap()
        .as_str()
        .unwrap()
        .trim()
        .to_lowercase();
    value["email_hash"] = json!(if email.is_empty() {
        String::new()
    } else {
        hex::encode(Md5::digest(email.as_bytes()))
    });
    value
}
pub async fn admin_list(app: &App, status: &str) -> Result<Value> {
    let status = if matches!(status, "approved" | "pending" | "spam") {
        status
    } else {
        "all"
    };
    let rows=sqlx::query("SELECT c.id,c.kind,c.target_id,c.author,c.site,c.content,c.created_at,c.status,c.reply_to,COALESCE(CASE WHEN c.kind='article' THEN (SELECT title FROM articles WHERE id=c.target_id) ELSE (SELECT substr(content,1,30) FROM posts WHERE id=c.target_id) END,'#'||c.target_id) AS target_label FROM comments c WHERE ?='all' OR status=? ORDER BY id DESC LIMIT 500").bind(status).bind(status).fetch_all(&app.db).await?;
    let mut counts = json!({"all":0,"approved":0,"pending":0,"spam":0});
    for (status, count) in
        sqlx::query_as::<_, (String, i64)>("SELECT status,COUNT(*) FROM comments GROUP BY status")
            .fetch_all(&app.db)
            .await?
    {
        counts["all"] = json!(counts["all"].as_i64().unwrap() + count);
        counts[status] = json!(count);
    }
    Ok(json!({"comments":rows.into_iter().map(row_json).collect::<Vec<_>>(),"counts":counts}))
}
#[derive(Deserialize)]
pub struct NewComment {
    pub kind: String,
    pub target: i64,
    #[serde(default)]
    pub website_url: String,
    #[serde(default)]
    pub author: String,
    #[serde(default)]
    pub email: String,
    #[serde(default)]
    pub site: String,
    #[serde(default)]
    pub content: String,
    #[serde(default)]
    pub reply_to: i64,
}
pub async fn submit(
    app: &App,
    peer: IpAddr,
    user_agent: &str,
    host: &str,
    mut input: NewComment,
) -> Result<Value> {
    let settings = Settings::load(app).await?;
    if settings.get("comment_provider") == "off" {
        return Err(Error::invalid("评论已关闭"));
    }
    if settings.get("comment_scope") == "article" && input.kind == "post" {
        return Err(Error::invalid("动态评论已关闭"));
    }
    if !matches!(input.kind.as_str(), "post" | "article") {
        return Err(Error::invalid("评论目标无效"));
    }
    if !input.website_url.trim().is_empty() {
        return Ok(json!({"ok":true}));
    }
    input.author = input.author.trim().to_owned();
    input.content = input.content.trim().to_owned();
    input.email = input.email.trim().to_owned();
    input.site = input.site.trim().to_owned();
    if input.author.is_empty() || input.author.chars().count() > 40 {
        return Err(Error::invalid("昵称必填且不超过 40 字"));
    }
    if input.content.is_empty() || input.content.chars().count() > 1000 {
        return Err(Error::invalid("评论内容必填且不超过 1000 字"));
    }
    if !input.email.is_empty()
        && (input.email.len() > 100
            || !input.email.split_once('@').is_some_and(|(local, domain)| {
                !local.is_empty()
                    && domain.contains('.')
                    && !domain.chars().any(char::is_whitespace)
            }))
    {
        return Err(Error::invalid("邮箱格式不正确"));
    }
    if !input.site.is_empty()
        && (input.site.chars().count() > 200
            || !(input.site.starts_with("http://") || input.site.starts_with("https://")))
    {
        return Err(Error::invalid("网址需以 http(s):// 开头"));
    }
    let _guard = app.content_lock.write().await;
    let mut tx = app.db.begin_with("BEGIN IMMEDIATE").await?;
    let exists: bool = sqlx::query_scalar(if input.kind == "post" {
        "SELECT EXISTS(SELECT 1 FROM posts WHERE id=?)"
    } else {
        "SELECT EXISTS(SELECT 1 FROM articles WHERE id=? AND draft=0)"
    })
    .bind(input.target)
    .fetch_one(&mut *tx)
    .await?;
    if !exists {
        return Err(Error::invalid("评论目标不存在"));
    }
    let hash = auth::digest(&format!("{peer}|{}", app.data.display()));
    let limit = settings.number("comment_rate_seconds");
    if limit > 0 {
        let since = (chrono::Utc::now() - chrono::Duration::seconds(limit))
            .with_timezone(&app.config.timezone)
            .to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
        let too_fast: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM comments WHERE ip_hash=? AND created_at>?)",
        )
        .bind(&hash)
        .bind(since)
        .fetch_one(&mut *tx)
        .await?;
        if too_fast {
            return Err(Error::invalid("评论太频繁了，稍等一分钟再试试"));
        }
    }
    let parent: Option<i64> = sqlx::query_scalar(
        "SELECT id FROM comments WHERE id=? AND kind=? AND target_id=? AND status='approved'",
    )
    .bind(input.reply_to)
    .bind(&input.kind)
    .bind(input.target)
    .fetch_optional(&mut *tx)
    .await?;
    let spam: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM comments WHERE ip_hash=? AND status='spam')",
    )
    .bind(&hash)
    .fetch_one(&mut *tx)
    .await?;
    let mut status = if spam || settings.get("comment_moderation") == "pre" {
        "pending"
    } else {
        "approved"
    };
    // Akismet（可选）：配了 akismet_key 才检查，判垃圾直接进 spam 且对提交方假成功
    let key = settings.get("akismet_key").trim().to_owned();
    if !key.is_empty() {
        let origin = if settings.get("site_url").is_empty() {
            format!("http://{host}")
        } else {
            settings.get("site_url").trim_end_matches('/').to_owned()
        };
        let body = {
            let mut form = url::form_urlencoded::Serializer::new(String::new());
            for (name, value) in [
                ("key", key.clone()),
                ("blog", origin),
                ("user_ip", peer.to_string()),
                (
                    "user_agent",
                    user_agent.chars().take(200).collect::<String>(),
                ),
                ("comment_type", "comment".into()),
                ("comment_author", input.author.clone()),
                ("comment_author_email", input.email.clone()),
                ("comment_content", input.content.clone()),
            ] {
                form.append_pair(name, &value);
            }
            form.finish().into_bytes()
        };
        let verdict = app
            .network
            .request(
                &format!("https://{key}.rest.akismet.com/1.1/comment-check"),
                "POST",
                &[(
                    "Content-Type".to_owned(),
                    "application/x-www-form-urlencoded".to_owned(),
                )],
                Some(body),
                None,
                std::time::Duration::from_secs(20),
                false,
                1024 * 1024,
                &CancellationToken::new(),
            )
            .await;
        if let Ok((_, body)) = verdict
            && String::from_utf8_lossy(&body).trim() == "true"
        {
            status = "spam";
        }
    }
    sqlx::query("INSERT INTO comments(kind,target_id,author,email,site,content,created_at,status,reply_to,ip_hash) VALUES(?,?,?,?,?,?,?,?,?,?)")
        .bind(&input.kind).bind(input.target).bind(&input.author).bind(&input.email).bind(&input.site).bind(&input.content).bind(app.config.now()).bind(status).bind(parent).bind(hash).execute(&mut *tx).await?;
    tx.commit().await?;
    // 邮件通知：失败只记日志，绝不打断提交（spam 不打扰管理员）
    let target_label: String = if input.kind == "article" {
        sqlx::query_scalar("SELECT COALESCE((SELECT title FROM articles WHERE id=?),'文章')")
            .bind(input.target)
            .fetch_one(&app.db)
            .await
            .unwrap_or_else(|_| "文章".into())
    } else {
        sqlx::query_scalar(
            "SELECT COALESCE((SELECT substr(content,1,30) FROM posts WHERE id=?),'动态')",
        )
        .bind(input.target)
        .fetch_one(&app.db)
        .await
        .unwrap_or_else(|_| "动态".into())
    };
    let origin = if settings.get("site_url").is_empty() {
        format!("http://{host}")
    } else {
        settings.get("site_url").trim_end_matches('/').to_owned()
    };
    let link = if input.kind == "article" {
        format!("{origin}/article/{}/", input.target)
    } else {
        format!("{origin}/#post-{}", input.target)
    };
    let notify_email = settings.get("notify_email").to_owned();
    if !notify_email.is_empty() && status != "spam" {
        let app = app.clone();
        let suffix = if status == "pending" {
            "（待审）"
        } else {
            ""
        };
        let subject = format!(
            "新评论{suffix}：{}",
            &input.author.chars().take(20).collect::<String>()
        );
        let body = format!(
            "{} 评论了「{}」：\n\n{}\n\n{}",
            input.author, target_label, input.content, link
        );
        tokio::spawn(async move {
            if let Err(error) = crate::mail::send(
                &app,
                &notify_email,
                &subject,
                &body,
                &CancellationToken::new(),
            )
            .await
            {
                tracing::warn!(%error, "评论通知邮件发送失败");
            }
        });
    }
    // 回复通知：对方留了邮箱、其评论已审、且本条直接放行时才发，避免垃圾外发
    if status == "approved"
        && let Some(parent) = parent
    {
        let parent: Option<(String, String)> =
            sqlx::query_as("SELECT author,email FROM comments WHERE id=? AND status='approved'")
                .bind(parent)
                .fetch_optional(&app.db)
                .await?;
        if let Some((parent_author, parent_email)) = parent
            && !parent_email.is_empty()
        {
            let app = app.clone();
            let subject = "你的评论收到了回复".to_owned();
            let body = format!(
                "{parent_author}，你好：\n\n{} 回复了你：\n\n{}\n\n{}",
                input.author, input.content, link
            );
            tokio::spawn(async move {
                if let Err(error) = crate::mail::send(
                    &app,
                    &parent_email,
                    &subject,
                    &body,
                    &CancellationToken::new(),
                )
                .await
                {
                    tracing::warn!(%error, "回复通知邮件发送失败");
                }
            });
        }
    }
    Ok(json!({"ok":true}))
}
pub async fn moderate(app: &App, id: i64, op: &str) -> Result<Value> {
    if !matches!(op, "spam" | "approve" | "delete") {
        return Err(Error::invalid("操作无效"));
    }
    let _guard = app.content_lock.write().await;
    let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM comments WHERE id=?)")
        .bind(id)
        .fetch_one(&app.db)
        .await?;
    if !exists {
        return Err(Error::not_found("评论不存在"));
    }
    if op == "delete" {
        sqlx::query("DELETE FROM comments WHERE id=? OR reply_to=?")
            .bind(id)
            .bind(id)
            .execute(&app.db)
            .await?;
    } else {
        sqlx::query("UPDATE comments SET status=? WHERE id=?")
            .bind(if op == "spam" { "spam" } else { "approved" })
            .bind(id)
            .execute(&app.db)
            .await?;
    }
    Ok(json!({"ok":true}))
}
fn valid_token(token: &str) -> bool {
    (16..=64).contains(&token.len())
        && token
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
pub async fn likes(app: &App, token: &str) -> Result<Value> {
    let liked: Vec<i64> = if valid_token(token) {
        sqlx::query_scalar("SELECT post_id FROM post_likes WHERE token=? ORDER BY post_id")
            .bind(token)
            .fetch_all(&app.db)
            .await?
    } else {
        Vec::new()
    };
    Ok(json!({"liked":liked}))
}
pub async fn like(app: &App, peer: IpAddr, id: i64, token: &str) -> Result<Value> {
    if !valid_token(token) {
        return Err(Error::invalid("访客标识无效"));
    }
    auth::rate_limit(&app.db, "like", &auth::digest(&peer.to_string()), 30, 60).await?;
    let _guard = app.content_lock.write().await;
    let mut tx = app.db.begin_with("BEGIN IMMEDIATE").await?;
    let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM posts WHERE id=?)")
        .bind(id)
        .fetch_one(&mut *tx)
        .await?;
    if !exists {
        return Err(Error::not_found("动态不存在"));
    }
    let removed = sqlx::query("DELETE FROM post_likes WHERE post_id=? AND token=?")
        .bind(id)
        .bind(token)
        .execute(&mut *tx)
        .await?
        .rows_affected()
        > 0;
    if !removed {
        sqlx::query("INSERT INTO post_likes(post_id,token,created_at) VALUES(?,?,?)")
            .bind(id)
            .bind(token)
            .bind(app.config.now())
            .execute(&mut *tx)
            .await?;
    }
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM post_likes WHERE post_id=?")
        .bind(id)
        .fetch_one(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(json!({"ok":true,"liked":!removed,"count":count}))
}

/// 静态导出用：按 term（`post:{id}` / `article:{id}`）查询 giscus 讨论的评论与点赞数。
/// 评论数为评论加回复，跨分页累加；无讨论时记零，远端异常则报错中止导出。
pub async fn giscus_counts(
    network: &crate::network::Client,
    repo: &str,
    terms: &[String],
    stop: &CancellationToken,
) -> Result<BTreeMap<String, Value>> {
    let mut queue = Vec::new();
    let mut seen = BTreeSet::new();
    for term in terms {
        if seen.insert(term.clone()) {
            queue.push((term.clone(), String::new()));
        }
    }
    let mut counts = BTreeMap::new();
    let mut jobs = tokio::task::JoinSet::new();
    while !queue.is_empty() || !jobs.is_empty() {
        while let Some((term, after)) = queue.pop() {
            let address = url::Url::parse_with_params(
                "https://giscus.app/api/discussions",
                &[
                    ("repo", repo),
                    ("term", term.as_str()),
                    ("strict", "true"),
                    ("first", "50"),
                    ("after", after.as_str()),
                ],
            )
            .map_err(|_| Error::invalid("giscus 统计查询失败"))?
            .to_string();
            let stop = stop.clone();
            let network = network.clone();
            jobs.spawn(async move {
                let result = network
                    .get_status(&address, false, 8 * 1024 * 1024, &stop)
                    .await;
                (term, after, result)
            });
        }
        let Some(joined) = jobs.join_next().await else {
            continue;
        };
        let (term, after, result) = joined.map_err(|_| Error::invalid("giscus 统计查询失败"))?;
        let (status, body) =
            result.map_err(|error| Error::invalid(format!("giscus 统计查询失败：{error}")))?;
        let data: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
        if status == 404 && data["error"] == "Discussion not found" && after.is_empty() {
            counts.insert(term, json!({"comments":0,"likes":0}));
            continue;
        }
        let discussion = &data["discussion"];
        let count = |value: &Value| value.as_i64().filter(|count| *count >= 0);
        let (Some(comments), Some(replies), Some(likes)) = (
            count(&discussion["totalCommentCount"]),
            count(&discussion["totalReplyCount"]),
            count(&discussion["reactions"]["HEART"]["count"]),
        ) else {
            return Err(Error::invalid(format!(
                "giscus 统计同步失败（{term}，HTTP {status}），请稍后重试"
            )));
        };
        let entry = counts
            .entry(term.clone())
            .or_insert_with(|| json!({"comments":0,"likes":0}));
        if after.is_empty() {
            entry["comments"] = json!(comments);
            entry["likes"] = json!(likes);
        }
        entry["comments"] = json!(entry["comments"].as_i64().unwrap() + replies);
        if discussion["pageInfo"]["hasNextPage"] == true {
            let cursor = discussion["pageInfo"]["endCursor"].as_str().unwrap_or("");
            if cursor.is_empty() || cursor == after {
                return Err(Error::invalid("giscus 统计分页游标无效"));
            }
            queue.push((term, cursor.to_owned()));
        }
    }
    Ok(counts)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::network::{self, TestResponses};
    use std::sync::{Arc, Mutex};

    fn address(term: &str, after: &str) -> String {
        url::Url::parse_with_params(
            "https://giscus.app/api/discussions",
            &[
                ("repo", "owner/repo"),
                ("term", term),
                ("strict", "true"),
                ("first", "50"),
                ("after", after),
            ],
        )
        .unwrap()
        .to_string()
    }

    fn discussion(comments: i64, replies: i64, likes: i64, cursor: &str) -> Vec<u8> {
        json!({"discussion":{"totalCommentCount":comments,"totalReplyCount":replies,"reactions":{"HEART":{"count":likes}},"pageInfo":{"hasNextPage":!cursor.is_empty(),"endCursor":cursor}}}).to_string().into_bytes()
    }

    #[tokio::test]
    async fn giscus_counts_merge_pages_and_zero_missing_discussions() {
        let fixture = Arc::new(Mutex::new(TestResponses::default()));
        let mut client = network::Client::new(None);
        client.fixture = Some(fixture.clone());
        {
            let mut responses = fixture.lock().unwrap();
            responses
                .responses
                .insert(address("post:1", ""), discussion(5, 2, 7, ""));
            responses.responses.insert(
                address("post:2", ""),
                br#"{"error":"Discussion not found"}"#.to_vec(),
            );
            responses.statuses.insert(address("post:2", ""), 404);
            responses
                .responses
                .insert(address("post:3", ""), discussion(3, 1, 9, "cursor-1"));
            responses
                .responses
                .insert(address("post:3", "cursor-1"), discussion(3, 4, 9, ""));
        }
        let stop = CancellationToken::new();
        let terms = ["post:1", "post:2", "post:3", "post:1"]
            .map(str::to_owned)
            .to_vec();
        let counts = giscus_counts(&client, "owner/repo", &terms, &stop)
            .await
            .unwrap();
        assert_eq!(counts.len(), 3);
        assert_eq!(counts["post:1"], json!({"comments":7,"likes":7}));
        assert_eq!(counts["post:2"], json!({"comments":0,"likes":0}));
        assert_eq!(counts["post:3"], json!({"comments":8,"likes":9}));
        let requests = fixture.lock().unwrap().requests.clone();
        assert_eq!(
            requests
                .iter()
                .filter(|(url, _)| url.contains("term=post%3A1"))
                .count(),
            1
        );
        assert!(requests.iter().all(|(url, _)| url.contains("strict=true")
            && url.contains("first=50")
            && url.contains("repo=owner%2Frepo")));
    }

    #[tokio::test]
    async fn giscus_counts_reject_invalid_responses_and_cancel() {
        let fixture = Arc::new(Mutex::new(TestResponses::default()));
        let mut client = network::Client::new(None);
        client.fixture = Some(fixture.clone());
        fixture
            .lock()
            .unwrap()
            .responses
            .insert(address("post:1", ""), b"server error".to_vec());
        fixture
            .lock()
            .unwrap()
            .statuses
            .insert(address("post:1", ""), 500);
        let stop = CancellationToken::new();
        let error = giscus_counts(&client, "owner/repo", &["post:1".to_owned()], &stop)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("giscus 统计同步失败"));
        {
            let mut responses = fixture.lock().unwrap();
            responses
                .responses
                .insert(address("post:2", ""), discussion(1, 1, 1, ""));
            responses
                .responses
                .insert(address("post:3", ""), discussion(1, 1, 1, "cursor-1"));
            responses.responses.insert(
                address("post:3", "cursor-1"),
                discussion(1, 1, 1, "cursor-1"),
            );
        }
        let error = giscus_counts(&client, "owner/repo", &["post:3".to_owned()], &stop)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("分页游标无效"));
        let missing = giscus_counts(
            &client,
            "owner/repo",
            &["post:404".to_owned()],
            &CancellationToken::new(),
        )
        .await
        .unwrap_err();
        assert!(missing.to_string().contains("夹具请求失败"));
        stop.cancel();
        let error = giscus_counts(&client, "owner/repo", &["post:2".to_owned()], &stop)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("任务已取消"));
    }
}

#[cfg(test)]
mod submit_tests {
    use super::*;
    use crate::{config::Config, database, network, plugins, tasks};
    use serde_json::json;
    use std::sync::{Arc, Mutex};

    #[tokio::test]
    async fn akismet_verdict_marks_spam_and_keeps_fake_success() {
        let root = tempfile::tempdir().unwrap();
        let data = root.path().join("data");
        let fixture = Arc::new(Mutex::new(network::TestResponses::default()));
        let mut client = network::Client::new(None);
        client.fixture = Some(fixture.clone());
        let app = App {
            db: database::open(&data).await.unwrap(),
            tasks: tasks::open(&root.path().join("work")).await.unwrap(),
            network: client,
            data,
            config: Arc::new(Config::default()),
            shutdown: CancellationToken::new(),
            password_slots: Arc::new(tokio::sync::Semaphore::new(2)),
            media_slots: Arc::new(tokio::sync::Semaphore::new(2)),
            content_lock: Arc::new(tokio::sync::RwLock::new(())),
        };
        plugins::migrate(&app).await.unwrap();
        for (key, value) in [
            ("akismet_key", "test-key"),
            ("notify_email", "admin@example.org"),
        ] {
            sqlx::query("INSERT INTO settings VALUES(?,?)")
                .bind(key)
                .bind(value)
                .execute(&app.db)
                .await
                .unwrap();
        }
        sqlx::query("INSERT INTO posts(content,source,images,meta,created_at,updated_at) VALUES('被评论的动态','','[]','{}','2026-11-26','2026-11-26')")
            .execute(&app.db)
            .await
            .unwrap();
        fixture.lock().unwrap().responses.insert(
            "https://test-key.rest.akismet.com/1.1/comment-check".into(),
            b"true".to_vec(),
        );
        let input = NewComment {
            kind: "post".into(),
            target: 1,
            website_url: String::new(),
            author: "访客".into(),
            email: "visitor@example.org".into(),
            site: String::new(),
            content: "疑似垃圾".into(),
            reply_to: 0,
        };
        let result = submit(
            &app,
            "127.0.0.1".parse().unwrap(),
            "fixture-agent",
            "127.0.0.1:8785",
            input,
        )
        .await
        .unwrap();
        assert_eq!(result, json!({"ok":true}));
        let status: String = sqlx::query_scalar("SELECT status FROM comments")
            .fetch_one(&app.db)
            .await
            .unwrap();
        assert_eq!(status, "spam");
        assert_eq!(fixture.lock().unwrap().requests.len(), 1);
        // 无 Akismet 配置时不外呼
        sqlx::query("UPDATE settings SET value='' WHERE key='akismet_key'")
            .execute(&app.db)
            .await
            .unwrap();
        let input = NewComment {
            kind: "post".into(),
            target: 1,
            website_url: String::new(),
            author: "访客".into(),
            email: String::new(),
            site: String::new(),
            content: "正常评论".into(),
            reply_to: 0,
        };
        submit(
            &app,
            "127.0.0.1".parse().unwrap(),
            "fixture-agent",
            "127.0.0.1:8785",
            input,
        )
        .await
        .unwrap();
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM comments")
            .fetch_one(&app.db)
            .await
            .unwrap();
        assert_eq!(count, 2);
        assert_eq!(fixture.lock().unwrap().requests.len(), 1);
    }
}

/// 导出用的 giscus 计数带缓存：`giscus_refresh_hours` 小时内的结果直接复用（0 表示每次都查），
/// 只对新增或过期的 term 发起网络查询。
pub async fn giscus_counts_cached(
    app: &App,
    terms: &[String],
    stop: &CancellationToken,
) -> Result<BTreeMap<String, Value>> {
    let settings = Settings::load(app).await?;
    let ttl = settings.number("giscus_refresh_hours").max(0) * 3600;
    let mut counts = BTreeMap::new();
    let mut stale: Vec<String> = Vec::new();
    if ttl > 0 {
        let now = chrono::Utc::now().timestamp();
        for term in terms {
            let row: Option<(String, String)> =
                sqlx::query_as("SELECT counts,fetched_at FROM giscus_counts WHERE term=?")
                    .bind(term)
                    .fetch_optional(&app.db)
                    .await?;
            let fresh = row
                .as_ref()
                .and_then(|(_, fetched)| chrono::DateTime::parse_from_rfc3339(fetched).ok())
                .map(|fetched| now - fetched.timestamp() < ttl)
                .unwrap_or(false);
            if fresh && let Some(value) = serde_json::from_str(&row.unwrap().0).ok() {
                counts.insert(term.clone(), value);
            } else {
                stale.push(term.clone());
            }
        }
    } else {
        stale = terms.to_vec();
    }
    if stale.is_empty() {
        return Ok(counts);
    }
    let fetched = giscus_counts(&app.network, &repo_of(&settings), &stale, stop).await?;
    let now = app.config.now();
    let mut tx = app.db.begin_with("BEGIN IMMEDIATE").await?;
    for term in &stale {
        if let Some(value) = fetched.get(term) {
            sqlx::query("INSERT INTO giscus_counts(term,counts,fetched_at) VALUES(?,?,?) ON CONFLICT(term) DO UPDATE SET counts=excluded.counts,fetched_at=excluded.fetched_at")
                .bind(term)
                .bind(value.to_string())
                .bind(&now)
                .execute(&mut *tx)
                .await?;
            counts.insert(term.clone(), value.clone());
        }
    }
    tx.commit().await?;
    Ok(counts)
}

fn repo_of(settings: &Settings) -> String {
    settings.get("giscus_repo").to_owned()
}

#[cfg(test)]
mod cache_tests {
    use super::*;
    use crate::{config::Config, database, network, plugins, tasks};
    use std::sync::{Arc, Mutex};

    #[tokio::test]
    async fn giscus_cache_reuses_fresh_rows_and_refetches_stale() {
        let root = tempfile::tempdir().unwrap();
        let data = root.path().join("data");
        let fixture = Arc::new(Mutex::new(network::TestResponses::default()));
        let mut client = network::Client::new(None);
        client.fixture = Some(fixture.clone());
        let app = App {
            db: database::open(&data).await.unwrap(),
            tasks: tasks::open(&root.path().join("work")).await.unwrap(),
            network: client,
            data,
            config: Arc::new(Config::default()),
            shutdown: CancellationToken::new(),
            password_slots: Arc::new(tokio::sync::Semaphore::new(2)),
            media_slots: Arc::new(tokio::sync::Semaphore::new(2)),
            content_lock: Arc::new(tokio::sync::RwLock::new(())),
        };
        plugins::migrate(&app).await.unwrap();
        for (key, value) in [
            ("comment_provider", "giscus"),
            ("giscus_repo", "owner/repo"),
            ("giscus_refresh_hours", "24"),
        ] {
            sqlx::query("INSERT INTO settings VALUES(?,?)")
                .bind(key)
                .bind(value)
                .execute(&app.db)
                .await
                .unwrap();
        }
        fixture.lock().unwrap().responses.insert(
            "https://giscus.app/api/discussions?repo=owner%2Frepo&term=post%3A1&strict=true&first=50&after="
                .into(),
            br#"{"discussion":{"totalCommentCount":1,"totalReplyCount":0,"reactions":{"HEART":{"count":2}},"pageInfo":{"hasNextPage":false}}}"#
                .to_vec(),
        );
        let terms = vec!["post:1".to_owned()];
        let stop = CancellationToken::new();
        let first = giscus_counts_cached(&app, &terms, &stop).await.unwrap();
        assert_eq!(first["post:1"], json!({"comments":1,"likes":2}));
        assert_eq!(fixture.lock().unwrap().requests.len(), 1);
        // TTL 内第二次查询直接复用缓存，不发请求
        let second = giscus_counts_cached(&app, &terms, &stop).await.unwrap();
        assert_eq!(second["post:1"], json!({"comments":1,"likes":2}));
        assert_eq!(fixture.lock().unwrap().requests.len(), 1);
        // 过期后重新查询
        sqlx::query("UPDATE giscus_counts SET fetched_at='2020-01-01T00:00:00+00:00'")
            .execute(&app.db)
            .await
            .unwrap();
        let third = giscus_counts_cached(&app, &terms, &stop).await.unwrap();
        assert_eq!(third["post:1"], json!({"comments":1,"likes":2}));
        assert_eq!(fixture.lock().unwrap().requests.len(), 2);
        // 间隔为 0：始终查询
        sqlx::query("UPDATE settings SET value='0' WHERE key='giscus_refresh_hours'")
            .execute(&app.db)
            .await
            .unwrap();
        giscus_counts_cached(&app, &terms, &stop).await.unwrap();
        assert_eq!(fixture.lock().unwrap().requests.len(), 3);
    }
}
