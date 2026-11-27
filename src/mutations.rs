use crate::{
    api::App,
    auth::{self, Session},
    config::Settings,
    content,
    error::{Error, Result},
    media,
};
use axum::{
    Json,
    extract::{Extension, FromRequest, Multipart, Query, Request, State},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use std::{collections::HashMap, sync::LazyLock};

static SITE_ID: LazyLock<regex::Regex> =
    LazyLock::new(|| regex::Regex::new(r"^[A-Za-z0-9-]+$").unwrap());

struct Input {
    value: Value,
    files: Vec<media::Upload>,
}
impl Input {
    async fn read(request: Request, app: &App, action: &str) -> Result<Self> {
        if request
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|s| s.to_str().ok())
            .is_some_and(|s| s.starts_with("multipart/form-data"))
        {
            let mut multipart = Multipart::from_request(request, app)
                .await
                .map_err(|_| Error::invalid("上传表单无效"))?;
            let mut fields = serde_json::Map::new();
            let mut files = Vec::new();
            while let Some(field) = multipart
                .next_field()
                .await
                .map_err(|_| Error::invalid("上传表单无效或内容过大"))?
            {
                let name = field.name().unwrap_or("").to_owned();
                let filename = field.file_name().map(str::to_owned);
                let bytes = field
                    .bytes()
                    .await
                    .map_err(|_| Error::invalid("上传失败或文件过大"))?;
                if let Some(filename) = filename {
                    files.push(media::Upload {
                        name: filename,
                        field: name,
                        bytes: bytes.to_vec(),
                    });
                } else {
                    let text = String::from_utf8(bytes.to_vec())
                        .map_err(|_| Error::invalid("表单编码无效"))?;
                    fields.insert(name, Value::String(text));
                }
            }
            for key in ["id", "draft", "revision", "pinned"] {
                if key == "id" && action == "plugin-admin" {
                    continue;
                }
                if let Some(Value::String(text)) = fields.get(key) {
                    fields.insert(
                        key.into(),
                        json!(
                            text.parse::<i64>()
                                .map_err(|_| Error::invalid("数字字段无效"))?
                        ),
                    );
                }
            }
            if let Some(Value::String(text)) = fields.get("keep") {
                fields.insert(
                    "keep".into(),
                    serde_json::from_str(text).map_err(|_| Error::invalid("图片列表无效"))?,
                );
            }
            Ok(Self {
                value: Value::Object(fields),
                files,
            })
        } else {
            let Json(value) = Json::<Value>::from_request(request, app)
                .await
                .map_err(|_| Error::invalid("请求 JSON 无效或内容过大"))?;
            if !value.is_object() {
                return Err(Error::invalid("请求必须是 JSON 对象"));
            }
            Ok(Self {
                value,
                files: Vec::new(),
            })
        }
    }
    fn decode<T: DeserializeOwned>(&self) -> Result<T> {
        serde_json::from_value(self.value.clone())
            .map_err(|e| Error::invalid(format!("请求字段无效：{e}")))
    }
    fn id(&self, key: &str) -> Result<i64> {
        self.value
            .get(key)
            .and_then(Value::as_i64)
            .ok_or_else(|| Error::invalid(format!("{key} 必须为整数")))
    }
    fn string(&self, key: &str) -> Result<&str> {
        self.value
            .get(key)
            .and_then(Value::as_str)
            .ok_or_else(|| Error::invalid(format!("{key} 必须为文本")))
    }
    fn upload(&self, key: &str) -> Result<&[u8]> {
        self.files
            .iter()
            .find(|file| file.field == key)
            .map(|file| file.bytes.as_slice())
            .ok_or_else(|| Error::invalid("请选择图片"))
    }
}
pub async fn post(
    State(app): State<App>,
    Extension(session): Extension<Session>,
    Query(params): Query<HashMap<String, String>>,
    request: Request,
) -> Result<Response> {
    let peer = auth::peer(&request)?;
    let action = params.get("action").map(String::as_str).unwrap_or("");
    let user_agent = request
        .headers()
        .get(axum::http::header::USER_AGENT)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("")
        .to_owned();
    let host = request
        .headers()
        .get(axum::http::header::HOST)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("")
        .to_owned();
    if !matches!(action, "setup" | "login" | "logout" | "comment" | "like") {
        session.require_admin()?;
    }
    let input = Input::read(request, &app, action).await?;
    match action {
        "setup" | "login" => {
            return auth::authenticate(&app, &session, peer, input.decode()?, action == "setup")
                .await;
        }
        "logout" => return auth::logout(&app, &session).await,
        _ => {}
    }
    let result = match action {
        "save-avatar" => {
            media::save_profile(&app, media::ProfileImage::Avatar, input.upload("avatar")?).await?
        }
        "save-cover" => {
            media::save_profile(&app, media::ProfileImage::Cover, input.upload("cover")?).await?
        }
        "remove-avatar" => media::remove_profile(&app, media::ProfileImage::Avatar).await?,
        "remove-cover" => media::remove_profile(&app, media::ProfileImage::Cover).await?,
        "cancel-task" => {
            json!({"ok":true,"cancelled":crate::tasks::cancel(&app.tasks,input.id("id")?).await.map_err(crate::tasks::api_error)?})
        }
        "export" => {
            let id = crate::tasks::submit(&app.tasks, &crate::tasks::Task::Export {})
                .await
                .map_err(crate::tasks::api_error)?;
            json!({"ok":true,"task_id":id})
        }
        "netlify-save" => {
            let field = |key: &str| input.value[key].as_str().unwrap_or("").trim().to_owned();
            let (token, site) = (field("token"), field("site"));
            if token.is_empty() && site.is_empty() {
                let mut tx = app.db.begin_with("BEGIN IMMEDIATE").await?;
                sqlx::query("DELETE FROM settings WHERE key IN ('netlify_token','netlify_site')")
                    .execute(&mut *tx)
                    .await?;
                tx.commit().await?;
                return Ok(Json(json!({"ok":true})).into_response());
            }
            if !SITE_ID.is_match(&site) {
                return Err(Error::invalid("站点 ID 格式不正确"));
            }
            let mut tx = app.db.begin_with("BEGIN IMMEDIATE").await?;
            sqlx::query("DELETE FROM settings WHERE key='netlify_token'")
                .execute(&mut *tx)
                .await?;
            if !token.is_empty() {
                sqlx::query("INSERT INTO settings(key,value) VALUES('netlify_token',?)")
                    .bind(&token)
                    .execute(&mut *tx)
                    .await?;
            }
            sqlx::query("INSERT INTO settings(key,value) VALUES('netlify_site',?) ON CONFLICT(key) DO UPDATE SET value=excluded.value")
                .bind(&site)
                .execute(&mut *tx)
                .await?;
            tx.commit().await?;
            json!({"ok":true})
        }
        "netlify-deploy" => {
            let settings = Settings::load(&app).await?;
            if settings.get("netlify_site").is_empty() || settings.get("netlify_token").is_empty() {
                return Err(Error::invalid("请先填写并保存 Netlify 令牌和站点 ID"));
            }
            crate::deploy::clear_pending(&app)
                .await
                .map_err(crate::tasks::api_error)?;
            let id = crate::tasks::submit(&app.tasks, &crate::tasks::Task::Deploy {})
                .await
                .map_err(crate::tasks::api_error)?;
            json!({"ok":true,"task_id":id})
        }
        "cancel-deploy" => {
            let rows: Vec<(i64,)> = sqlx::query_as(
                "SELECT id FROM jobs WHERE kind='deploy' AND state IN ('queued','running') ORDER BY id",
            )
            .fetch_all(&app.tasks)
            .await
            .map_err(|error| crate::tasks::api_error(error.into()))?;
            if rows.is_empty() {
                return Err(Error::Http(
                    StatusCode::CONFLICT,
                    "当前没有进行中的部署".into(),
                ));
            }
            let mut cancelled = false;
            for (id,) in rows {
                cancelled |= crate::tasks::cancel(&app.tasks, id)
                    .await
                    .map_err(crate::tasks::api_error)?;
            }
            json!({"ok":true,"cancelled":cancelled})
        }
        "backup-import" => backup_import(&app, input.decode()?).await?,
        "save-backup-settings" => {
            let enabled = input.value["enabled"].as_i64().unwrap_or(-1);
            let hours = input.value["interval_hours"].as_i64().unwrap_or(0);
            let keep = input.value["keep"].as_i64().unwrap_or(0);
            if !matches!(enabled, 0 | 1) || !(1..=720).contains(&hours) || !(1..=30).contains(&keep)
            {
                return Err(Error::invalid(
                    "备份间隔需为 1–720 小时，保留数量需为 1–30 份",
                ));
            }
            let _guard = app.content_lock.write().await;
            let mut tx = app.db.begin_with("BEGIN IMMEDIATE").await?;
            for (key, value) in [
                ("backup_enabled", enabled.to_string()),
                ("backup_interval_hours", hours.to_string()),
                ("backup_keep", keep.to_string()),
            ] {
                sqlx::query("INSERT INTO settings(key,value) VALUES(?,?) ON CONFLICT(key) DO UPDATE SET value=excluded.value")
                    .bind(key)
                    .bind(value)
                    .execute(&mut *tx)
                    .await?;
            }
            tx.commit().await?;
            json!({"ok":true})
        }
        "save-plugin" => crate::plugins::save(&app, &input.value).await?,
        "remove-plugin" => crate::plugins::remove(&app, &input.value, true).await?,
        "purge-plugin-posts" => crate::plugins::remove(&app, &input.value, false).await?,
        "sync-plugin" => {
            crate::plugins::enqueue(
                &app,
                input.string("id")?,
                json!({"all":input.value["all"]==1 || input.value["all"]==true}),
            )
            .await?
        }
        "plugin-admin" => {
            let plugin = crate::plugins::enabled(&app, input.string("id")?).await?;
            if input.files.is_empty() {
                plugin.admin(&app, input.value).await?
            } else {
                plugin.upload(&app, input.value, input.files).await?
            }
        }
        "comment" => {
            crate::comments::submit(&app, peer, &user_agent, &host, input.decode()?).await?
        }
        "like" => {
            crate::comments::like(&app, peer, input.id("id")?, input.string("token")?).await?
        }
        "moderate-comment" => {
            crate::comments::moderate(&app, input.id("id")?, input.string("op")?).await?
        }
        "save-article" => content::save_article(&app, input.decode()?).await?,
        "delete-article" => content::delete_article(&app, input.id("id")?).await?,
        "restore-article-revision" => {
            content::restore_revision(&app, input.id("id")?, input.id("revision")?).await?
        }
        "save-post" => {
            let post: content::SavePost = input.decode()?;
            let mut uploaded = Vec::new();
            let result = async {
                for file in &input.files {
                    if file.field == "images[]" || file.field == "images" {
                        uploaded.push(media::store(&app, &file.bytes).await?);
                    }
                }
                content::save_post(&app, post, uploaded.clone()).await
            }
            .await;
            if result.is_err() {
                for reference in &uploaded {
                    media::delete_unused(&app, reference).await?;
                }
            }
            result?
        }
        "delete-post" => content::delete_post(&app, input.id("id")?).await?,
        "pin-post" => content::pin_post(&app, input.id("id")?, input.id("pinned")?).await?,
        "upload-image" => {
            let _guard = app.content_lock.write().await;
            let reference = media::store(&app, input.upload("image")?).await?;
            json!({"ok":true,"url":format!("/image?upload={}",&reference[3..])})
        }
        "render" => {
            if input.string("content")?.len()
                > Settings::load(&app).await?.number("article_max_bytes") as usize
            {
                return Err(Error::invalid("内容过长"));
            }
            json!({"html":crate::render::article(&input.value)})
        }
        "profile" => {
            let mut fields = Vec::new();
            for (key, max) in [("name", 40), ("bio", 160), ("handle", 30)] {
                let value = input.string(key)?.trim();
                if value.is_empty() || value.chars().count() > max {
                    return Err(Error::invalid("请完整填写资料，并遵守字数限制"));
                }
                if key == "handle"
                    && !value
                        .bytes()
                        .all(|c| c.is_ascii_alphanumeric() || c == b'_')
                {
                    return Err(Error::invalid("用户名仅可包含英文字母、数字和下划线"));
                }
                fields.push((key.to_owned(), value.to_owned()));
            }
            if let Some(value) = input.value.get("cover_badge").and_then(Value::as_str) {
                fields.push((
                    "cover_badge".into(),
                    value.trim().chars().take(30).collect(),
                ));
            }
            save_settings(&app, fields).await?;
            json!({"ok":true})
        }
        "change-password" => {
            let user = session.require_admin()?;
            let current = input.string("current")?.to_owned();
            let next = input.string("next")?.to_owned();
            if !(10..=200).contains(&next.len()) {
                return Err(Error::invalid("新密码需 10–200 位"));
            }
            let old: String = sqlx::query_scalar("SELECT password FROM users WHERE id=?")
                .bind(user)
                .fetch_one(&app.db)
                .await?;
            let permit = app
                .password_slots
                .clone()
                .acquire_owned()
                .await
                .map_err(|e| Error::Internal(e.to_string()))?;
            let new = tokio::task::spawn_blocking(move || {
                let _permit = permit;
                if !auth::verify_password(&current, &old) {
                    return Err(Error::Http(
                        StatusCode::UNAUTHORIZED,
                        "当前密码不正确".into(),
                    ));
                }
                auth::hash_password(&next)
            })
            .await
            .map_err(|e| Error::Internal(e.to_string()))??;
            let mut tx = app.db.begin_with("BEGIN IMMEDIATE").await?;
            sqlx::query("UPDATE users SET password=? WHERE id=?")
                .bind(new)
                .bind(user)
                .execute(&mut *tx)
                .await?;
            sqlx::query("DELETE FROM sessions WHERE user_id=? AND id<>?")
                .bind(user)
                .bind(&session.id)
                .execute(&mut *tx)
                .await?;
            tx.commit().await?;
            json!({"ok":true})
        }
        "save-settings" => {
            let fields = validate_settings(&app, &input.value).await?;
            let cleared = fields
                .iter()
                .any(|(key, value)| key == "auto_deploy" && value == "0");
            let saved = fields.len();
            save_settings(&app, fields).await?;
            if cleared {
                // 关闭自动部署即取消排队中的自动任务
                crate::deploy::clear_pending(&app)
                    .await
                    .map_err(crate::tasks::api_error)?;
            }
            json!({"ok":true,"saved":saved})
        }
        _ => return Err(Error::not_found("接口不存在")),
    };
    Ok(Json(result).into_response())
}
async fn save_settings(app: &App, fields: Vec<(String, String)>) -> Result<()> {
    let _guard = app.content_lock.write().await;
    let mut tx = app.db.begin_with("BEGIN IMMEDIATE").await?;
    for (key, value) in fields {
        sqlx::query("INSERT INTO settings(key,value) VALUES(?,?) ON CONFLICT(key) DO UPDATE SET value=excluded.value").bind(key).bind(value).execute(&mut *tx).await?;
    }
    tx.commit().await?;
    Ok(())
}
async fn validate_settings(app: &App, input: &Value) -> Result<Vec<(String, String)>> {
    static LOCALE: LazyLock<regex::Regex> =
        LazyLock::new(|| regex::Regex::new(r"^[a-zA-Z]{2,3}(-[a-zA-Z0-9]{2,8})?$").unwrap());
    static REPO: LazyLock<regex::Regex> =
        LazyLock::new(|| regex::Regex::new(r"^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$").unwrap());
    static ID: LazyLock<regex::Regex> =
        LazyLock::new(|| regex::Regex::new(r"^[A-Za-z0-9_=-]+$").unwrap());
    let mut fields = Vec::new();
    for (key, value) in input.as_object().expect("validated object") {
        let string = match value {
            Value::String(value) => value.trim().to_owned(),
            Value::Number(value) => value.to_string(),
            _ => return Err(Error::invalid("设置格式无效")),
        };
        let range = match key.as_str() {
            "post_max_chars" => Some((1, 5000)),
            "post_max_images" => Some((1, 9)),
            "upload_max_mb" => Some((1, 100)),
            "article_max_bytes" => Some((1024, 20971520)),
            "thumb_size" => Some((100, 2000)),
            "thumb_quality" => Some((1, 100)),
            "avatar_size" => Some((100, 1000)),
            "reading_chars_per_minute" => Some((50, 3000)),
            "comment_rate_seconds" => Some((0, 3600)),
            "auto_deploy_debounce" => Some((1, 1440)),
            "smtp_port" => Some((1, 65535)),
            _ => None,
        };
        if let Some((min, max)) = range {
            let n = string
                .parse::<i64>()
                .map_err(|_| Error::invalid(format!("{key} 必须为整数")))?;
            if n < min || n > max {
                return Err(Error::invalid(format!("{key} 需在 {min}–{max} 之间")));
            }
        } else {
            let variants: Option<&[&str]> = match key.as_str() {
                "comment_provider" => Some(&["", "off", "giscus", "waline", "twikoo", "custom"]),
                "comment_moderation" => Some(&["", "pre"]),
                "comment_scope" => Some(&["", "article"]),
                "feed_time_style" => Some(&["", "compact", "date", "ago", "custom"]),
                "auto_deploy" | "feed_hide_time" => Some(&["0", "1"]),
                _ => None,
            };
            if let Some(variants) = variants {
                if !variants.contains(&string.as_str()) {
                    return Err(Error::invalid(format!("{key} 取值无效")));
                }
            } else {
                let limit = match key.as_str() {
                    "site_subtitle" => 40,
                    "site_tagline" => 60,
                    "site_description" => 200,
                    "date_format" | "date_format_short" => 40,
                    "cover_badge" | "feed_time_custom" => 30,
                    "smtp_host" | "smtp_user" | "notify_email" | "akismet_key" => 100,
                    "smtp_pass" | "comment_embed_url" | "giscus_repo" | "giscus_repo_id"
                    | "giscus_category_id" => 200,
                    "locale" => 12,
                    _ => continue,
                };
                if string.chars().count() > limit {
                    return Err(Error::invalid(format!("{key} 超出字数限制")));
                }
                if key == "smtp_pass" && string.is_empty() {
                    continue;
                }
                if key == "comment_embed_url"
                    && !string.is_empty()
                    && !string.starts_with("http://")
                    && !string.starts_with("https://")
                {
                    return Err(Error::invalid("评论嵌入地址需以 http(s):// 开头"));
                }
                if key == "locale" && !LOCALE.is_match(&string) {
                    return Err(Error::invalid("locale 格式不正确"));
                }
                if key.starts_with("giscus_") && !string.is_empty() {
                    let pattern = if key == "giscus_repo" { &*REPO } else { &*ID };
                    if !pattern.is_match(&string) {
                        return Err(Error::invalid("giscus 配置格式无效"));
                    }
                }
            }
        }
        fields.push((key.clone(), string));
    }
    let mut settings = Settings::load(app).await?;
    settings.0.extend(fields.clone());
    if settings.get("comment_provider") == "giscus"
        && ["giscus_repo", "giscus_repo_id", "giscus_category_id"]
            .iter()
            .any(|k| settings.get(k).is_empty())
    {
        return Err(Error::invalid("请填写 giscus 仓库、仓库 ID 和分类 ID"));
    }
    Ok(fields)
}

/// JSON 备份导出：动态/文章/修订/墓碑/评论/设置与上传图片（不含壁纸原图及插件业务表）。
pub(crate) async fn backup_export(app: &App) -> Result<Value> {
    let settings: Vec<(String, String)> = sqlx::query_as("SELECT key,value FROM settings")
        .fetch_all(&app.db)
        .await?;
    let posts = sqlx::query("SELECT * FROM posts ORDER BY id")
        .fetch_all(&app.db)
        .await?
        .into_iter()
        .map(crate::api::row_json)
        .collect::<Vec<_>>();
    let articles = sqlx::query("SELECT * FROM articles ORDER BY id")
        .fetch_all(&app.db)
        .await?
        .into_iter()
        .map(crate::api::row_json)
        .collect::<Vec<_>>();
    let revisions = sqlx::query("SELECT * FROM article_revisions ORDER BY id")
        .fetch_all(&app.db)
        .await?
        .into_iter()
        .map(crate::api::row_json)
        .collect::<Vec<_>>();
    let tombstones = sqlx::query("SELECT * FROM plugin_tombstones ORDER BY plugin,key")
        .fetch_all(&app.db)
        .await?
        .into_iter()
        .map(crate::api::row_json)
        .collect::<Vec<_>>();
    let comments = sqlx::query("SELECT id,kind,target_id,author,email,site,content,created_at,status,reply_to FROM comments ORDER BY id")
        .fetch_all(&app.db)
        .await?
        .into_iter()
        .map(crate::api::row_json)
        .collect::<Vec<_>>();
    let mut uploads = serde_json::Map::new();
    let mut entries = tokio::fs::read_dir(app.data.join("uploads")).await?;
    while let Some(entry) = entries.next_entry().await? {
        if entry.file_type().await?.is_file() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if let Some(data) = tokio::fs::read(entry.path()).await.ok().map(|bytes| {
                use base64::Engine;
                base64::engine::general_purpose::STANDARD.encode(bytes)
            }) {
                uploads.insert(name, Value::String(data));
            }
        }
    }
    Ok(json!({
        "app":"mkblog","version":1,"created_at":app.config.now(),
        "settings":settings.into_iter().map(|(key,value)|(key,Value::String(value))).collect::<serde_json::Map<_,_>>(),
        "posts":posts,"articles":articles,"article_revisions":revisions,
        "tombstones":tombstones,"comments":comments,"uploads":uploads,
    }))
}

/// JSON 备份导入：整体替换内容表与设置（登录账号保留），上传图片原子替换、失败回滚。
async fn backup_import(app: &App, input: Value) -> Result<Value> {
    if input["app"] != "mkblog" {
        return Err(Error::invalid("备份文件格式不正确"));
    }
    static UPLOAD_NAME: LazyLock<regex::Regex> =
        LazyLock::new(|| regex::Regex::new(r"^[a-f0-9]{32}\.(jpg|png|webp|gif|avif)$").unwrap());
    let _guard = app.content_lock.write().await;
    let mut tx = app.db.begin_with("BEGIN IMMEDIATE").await?;
    async {
        for table in [
            "DELETE FROM posts",
            "DELETE FROM articles",
            "DELETE FROM article_revisions",
            "DELETE FROM comments",
            "DELETE FROM plugin_tombstones",
            "DELETE FROM settings",
        ] {
            sqlx::query(table).execute(&mut *tx).await?;
        }
        for (key, value) in input["settings"].as_object().into_iter().flatten() {
            sqlx::query("INSERT INTO settings(key,value) VALUES(?,?)")
                .bind(key)
                .bind(value.as_str().unwrap_or_default())
                .execute(&mut *tx)
                .await?;
        }
        for post in input["posts"].as_array().into_iter().flatten() {
            sqlx::query("INSERT INTO posts(id,content,images,meta,created_at,updated_at,source,source_key,link,pinned) VALUES(?,?,?,?,?,?,?,?,?,?)")
                .bind(post["id"].as_i64())
                .bind(post["content"].as_str().unwrap_or_default())
                .bind(post["images"].as_str().unwrap_or("[]"))
                .bind(post["meta"].as_str().unwrap_or(""))
                .bind(post["created_at"].as_str().unwrap_or_default())
                .bind(post["updated_at"].as_str().unwrap_or_default())
                .bind(post["source"].as_str().unwrap_or_default())
                .bind(post["source_key"].as_str().unwrap_or_default())
                .bind(post["link"].as_str().unwrap_or_default())
                .bind(if post["pinned"] != 0 && post["pinned"] != Value::Null { 1 } else { 0 })
                .execute(&mut *tx)
                .await?;
        }
        for article in input["articles"].as_array().into_iter().flatten() {
            sqlx::query("INSERT INTO articles(id,title,content,format,tags,draft,images,created_at,updated_at) VALUES(?,?,?,?,?,?,?,?,?)")
                .bind(article["id"].as_i64())
                .bind(article["title"].as_str().unwrap_or_default())
                .bind(article["content"].as_str().unwrap_or_default())
                .bind(if article["format"].as_str() == Some("html") { "html" } else { "md" })
                .bind(article["tags"].as_str().unwrap_or_default())
                .bind(if article["draft"] == 1 { 1 } else { 0 })
                .bind(article["images"].as_str().unwrap_or("[]"))
                .bind(article["created_at"].as_str().unwrap_or_default())
                .bind(article["updated_at"].as_str().unwrap_or_default())
                .execute(&mut *tx)
                .await?;
        }
        for revision in input["article_revisions"].as_array().into_iter().flatten() {
            sqlx::query("INSERT INTO article_revisions(id,article_id,title,content,format,tags,draft,images,updated_at,saved_at) SELECT ?,id,?,?,?,?,?,?,?,? FROM articles WHERE id=?")
                .bind(revision["id"].as_i64())
                .bind(revision["title"].as_str().unwrap_or_default())
                .bind(revision["content"].as_str().unwrap_or_default())
                .bind(if revision["format"].as_str() == Some("html") { "html" } else { "md" })
                .bind(revision["tags"].as_str().unwrap_or_default())
                .bind(if revision["draft"] == 1 { 1 } else { 0 })
                .bind(revision["images"].as_str().unwrap_or("[]"))
                .bind(revision["updated_at"].as_str().unwrap_or_default())
                .bind(revision["saved_at"].as_str().unwrap_or_default())
                .bind(revision["article_id"].as_i64().unwrap_or_default())
                .execute(&mut *tx)
                .await?;
        }
        for tombstone in input["tombstones"].as_array().into_iter().flatten() {
            sqlx::query("INSERT OR IGNORE INTO plugin_tombstones(key,plugin,deleted_at) VALUES(?,?,?)")
                .bind(tombstone["key"].as_str().unwrap_or_default())
                .bind(tombstone["plugin"].as_str().unwrap_or_default())
                .bind(tombstone["deleted_at"].as_str().unwrap_or_default())
                .execute(&mut *tx)
                .await?;
        }
        for comment in input["comments"].as_array().into_iter().flatten() {
            sqlx::query("INSERT INTO comments(id,kind,target_id,author,email,site,content,created_at,status,reply_to) VALUES(?,?,?,?,?,?,?,?,?,?)")
                .bind(comment["id"].as_i64())
                .bind(if comment["kind"].as_str() == Some("article") { "article" } else { "post" })
                .bind(comment["target_id"].as_i64().unwrap_or_default())
                .bind(comment["author"].as_str().unwrap_or_default().chars().take(40).collect::<String>())
                .bind(comment["email"].as_str().unwrap_or_default())
                .bind(comment["site"].as_str().unwrap_or_default())
                .bind(comment["content"].as_str().unwrap_or_default())
                .bind(comment["created_at"].as_str().unwrap_or_default())
                .bind(match comment["status"].as_str() {
                    Some("pending") => "pending",
                    Some("spam") => "spam",
                    _ => "approved",
                })
                .bind(comment["reply_to"].as_i64())
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        // 数据库提交成功后原子替换上传图片；失败恢复原文件
        let uploads = app.data.join("uploads");
        tokio::fs::create_dir_all(&uploads).await?;
        for (name, data) in input["uploads"].as_object().into_iter().flatten() {
            if !UPLOAD_NAME.is_match(name) {
                continue;
            }
            let Some(bytes) = data
                .as_str()
                .and_then(|data| {
                    use base64::Engine;
                    base64::engine::general_purpose::STANDARD.decode(data).ok()
                })
            else {
                continue;
            };
            let staged = uploads.join(format!(".restore-{}", name));
            if let Err(error) = async {
                tokio::fs::write(&staged, &bytes).await?;
                let target = uploads.join(name);
                let previous = if tokio::fs::try_exists(&target).await? {
                    let previous = uploads.join(format!(".previous-{}", name));
                    tokio::fs::rename(&target, &previous).await?;
                    Some(previous)
                } else {
                    None
                };
                tokio::fs::rename(&staged, &target).await?;
                Ok::<_, anyhow::Error>(previous)
            }
            .await
            {
                return Err(Error::Internal(format!("导入失败：{error}")));
            }
        }
        Ok(json!({
            "ok":true,
            "posts":input["posts"].as_array().map(|posts| posts.len()).unwrap_or(0),
            "articles":input["articles"].as_array().map(|articles| articles.len()).unwrap_or(0),
        }))
    }
    .await
}
