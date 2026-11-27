use crate::{
    auth::Session,
    config::{Config, Settings},
    error::{Error, Result},
};
use axum::{
    Json,
    extract::{Extension, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::{Value, json};
use sqlx::{Column, Row, SqlitePool, TypeInfo, ValueRef, sqlite::SqliteRow};
use std::{collections::HashMap, path::PathBuf, sync::Arc};

#[derive(Clone)]
pub struct App {
    pub db: SqlitePool,
    pub tasks: SqlitePool,
    pub network: crate::network::Client,
    pub shutdown: tokio_util::sync::CancellationToken,
    pub data: PathBuf,
    pub config: Arc<Config>,
    pub password_slots: Arc<tokio::sync::Semaphore>,
    pub media_slots: Arc<tokio::sync::Semaphore>,
    pub content_lock: Arc<tokio::sync::RwLock<()>>,
}

pub(crate) fn row_json(row: SqliteRow) -> Value {
    let mut object = serde_json::Map::new();
    for col in row.columns() {
        let value = match row.try_get_raw(col.ordinal()).unwrap().type_info().name() {
            "INTEGER" => json!(row.get::<Option<i64>, _>(col.ordinal())),
            "REAL" => json!(row.get::<Option<f64>, _>(col.ordinal())),
            _ => json!(row.get::<Option<String>, _>(col.ordinal())),
        };
        object.insert(col.name().into(), value);
    }
    Value::Object(object)
}
fn text<'a>(params: &'a HashMap<String, String>, name: &str) -> &'a str {
    params.get(name).map(String::as_str).unwrap_or("")
}
fn cursor(params: &HashMap<String, String>, post: bool) -> Result<Option<Vec<Value>>> {
    let raw = text(params, "cursor");
    if raw.is_empty() {
        return Ok(None);
    }
    let value = STANDARD
        .decode(raw)
        .ok()
        .and_then(|b| serde_json::from_slice::<Vec<Value>>(&b).ok());
    let valid = value.as_ref().is_some_and(|v| {
        if post {
            v.len() == 3
                && matches!(v[0].as_i64(), Some(0 | 1))
                && v[1].is_string()
                && v[2].is_i64()
        } else {
            v.len() == 2 && v[0].is_string() && v[1].is_i64()
        }
    });
    if !valid {
        return Err(Error::Http(
            StatusCode::UNPROCESSABLE_ENTITY,
            "分页游标无效".into(),
        ));
    }
    Ok(value)
}
fn next_cursor(items: &[Value], more: bool, post: bool) -> Value {
    if !more {
        return Value::Null;
    }
    let last = items.last().unwrap();
    let values = if post {
        json!([last["pinned"], last["created_at"], last["id"]])
    } else {
        json!([last["created_at"], last["id"]])
    };
    json!(STANDARD.encode(values.to_string()))
}

pub async fn api(
    State(app): State<App>,
    Query(params): Query<HashMap<String, String>>,
    Extension(session): Extension<Session>,
) -> Result<axum::response::Response> {
    use axum::response::IntoResponse;
    let action = params.get("action").map(String::as_str).unwrap_or("state");
    if action == "download-static" {
        return static_site_zip(&app, &session).await;
    }
    let value = match action {
        "state" => state(&app, &session).await?,
        "posts" => posts_filtered(&app, &params, None).await?,
        "backup-export" => {
            session.require_admin()?;
            crate::mutations::backup_export(&app).await?
        }
        "tasks" => {
            session.require_admin()?;
            let settings = Settings::load(&app).await?;
            let next_run: Option<i64> =
                sqlx::query_scalar("SELECT next_run FROM schedules WHERE key='backup'")
                    .fetch_optional(&app.tasks)
                    .await
                    .map_err(|error| crate::tasks::api_error(error.into()))?;
            json!({
                "jobs":crate::tasks::list(&app.tasks).await.map_err(crate::tasks::api_error)?,
                "backup":{
                    "enabled":settings.enabled("backup_enabled"),
                    "interval_hours":settings.number("backup_interval_hours").max(1),
                    "keep":settings.number("backup_keep").max(1),
                    "next_run":next_run,
                    "directory":app.data.join("backups"),
                    "files":crate::backup::files(&app).await.map_err(crate::tasks::api_error)?,
                }
            })
        }
        "deploy-status" => {
            session.require_admin()?;
            let row = sqlx::query("SELECT id,state,cancel_requested,phase,completed,total,result,error FROM jobs WHERE kind='deploy' ORDER BY id DESC LIMIT 1")
                .fetch_optional(&app.tasks)
                .await
                .map_err(|error| crate::tasks::api_error(error.into()))?;
            let status = row.map(|row| {
                let mut job = crate::api::row_json(row);
                job["cancel_requested"] = json!(job["cancel_requested"] == 1);
                job["result"] = job["result"]
                    .as_str()
                    .and_then(|s| serde_json::from_str(s).ok())
                    .unwrap_or(Value::Null);
                let phase = job["phase"].as_str().unwrap_or("").to_owned();
                match job["state"].as_str() {
                    Some("queued" | "running") => {
                        let (state, current) = if phase.starts_with("上传") {
                            ("uploading", phase.trim_start_matches("上传").trim().to_owned())
                        } else if phase == "等待 Netlify 构建" {
                            ("waiting", String::new())
                        } else {
                            ("exporting", String::new())
                        };
                        json!({
                            "state": state,
                            "files_done": job["completed"],
                            "files_total": if job["total"].is_null() { 0 } else { job["total"].as_i64().unwrap() },
                            "current": current,
                        })
                    }
                    Some("succeeded") => json!({"state":"done","url":job["result"]["url"]}),
                    Some("cancelled" | "interrupted") => {
                        json!({"state":"cancelled","error":job["error"]})
                    }
                    _ => json!({"state":"error","error":job["error"]}),
                }
            });
            let cancelling = sqlx::query_scalar::<_, i64>(
                "SELECT EXISTS(SELECT 1 FROM jobs WHERE kind='deploy' AND state='running' AND cancel_requested=1)",
            )
            .fetch_one(&app.tasks)
            .await
            .map_err(|error| crate::tasks::api_error(error.into()))?
                != 0;
            let pending: Option<i64> =
                sqlx::query_scalar("SELECT due_at FROM deferred WHERE key='auto-deploy'")
                    .fetch_optional(&app.tasks)
                    .await
                    .map_err(|error| crate::tasks::api_error(error.into()))?;
            let pending = pending.map(|due_at| {
                json!({"due_at": due_at, "in": (due_at - chrono::Utc::now().timestamp()).max(0)})
            });
            json!({"status":status,"cancelling":cancelling,"pending":pending})
        }
        "plugins" => {
            session.require_admin()?;
            crate::plugins::admin_list(&app).await?
        }
        "plugin-data" => {
            let plugin = crate::plugins::get(text(&params, "id"))?;
            if !crate::plugins::config(plugin, &Settings::load(&app).await?).enabled("_enabled") {
                return Err(Error::not_found("插件未启用"));
            }
            json!({"ok":true,"data":plugin.data(&app,&params,session.user.is_some()).await?})
        }
        "comments" => {
            crate::comments::list(
                &app,
                text(&params, "kind"),
                text(&params, "target").parse().unwrap_or(0),
            )
            .await?
        }
        "likes" => crate::comments::likes(&app, text(&params, "token")).await?,
        "comments-admin" => {
            session.require_admin()?;
            crate::comments::admin_list(&app, text(&params, "status")).await?
        }
        "articles" => articles(&app, &params, session.user.is_some()).await?,
        "article-revisions" => {
            session.require_admin()?;
            crate::content::revisions(
                &app,
                text(&params, "id").parse().unwrap_or(0),
                params.get("revision").map(|s| s.parse().unwrap_or(0)),
                text(&params, "cursor").parse().unwrap_or(0),
            )
            .await?
        }
        _ => return Err(Error::Http(StatusCode::NOT_FOUND, "接口不存在".into())),
    };
    Ok(Json(value).into_response())
}

/// 打包最近一次静态导出为 ZIP 下载；缓存比产物新时直接复用。
async fn static_site_zip(app: &App, session: &Session) -> Result<axum::response::Response> {
    use axum::response::IntoResponse;
    session.require_admin()?;
    if !app.config.export_dir.join("index.html").is_file() {
        return Err(Error::Http(
            StatusCode::UNPROCESSABLE_ENTITY,
            "还没有生成静态站点，请先点「生成静态站点」".into(),
        ));
    }
    let zip = app.data.join("static-site.zip");
    let export_dir = app.config.export_dir.clone();
    let stale = {
        let zip = zip.clone();
        tokio::task::spawn_blocking(move || {
            let newest = crate::archive::newest_modified(&export_dir)?;
            Ok::<_, std::io::Error>(
                std::fs::metadata(&zip)
                    .and_then(|metadata| metadata.modified())
                    .map(|time| time < newest)
                    .unwrap_or(true),
            )
        })
        .await
        .map_err(|error| crate::tasks::api_error(error.into()))?
        .map_err(|error| crate::tasks::api_error(error.into()))?
    };
    if stale {
        let export_dir = app.config.export_dir.clone();
        let zip = zip.clone();
        tokio::task::spawn_blocking(move || crate::archive::zip_tree(&export_dir, &zip))
            .await
            .map_err(|error| crate::tasks::api_error(error.into()))?
            .map_err(crate::tasks::api_error)?;
    }
    let bytes = tokio::fs::read(&zip)
        .await
        .map_err(|error| crate::tasks::api_error(error.into()))?;
    let mut response = bytes.into_response();
    response.headers_mut().insert(
        axum::http::header::CONTENT_TYPE,
        axum::http::HeaderValue::from_static("application/zip"),
    );
    response.headers_mut().insert(
        axum::http::header::CONTENT_DISPOSITION,
        axum::http::HeaderValue::from_static("attachment; filename=\"muke-static-site.zip\""),
    );
    Ok(response)
}

pub(crate) async fn public_state(app: &App, settings: &Settings) -> Result<Value> {
    let profile = |key: &str, default: &str| {
        let value = settings.get(key);
        if value.is_empty() {
            settings.get(default).to_owned()
        } else {
            value.to_owned()
        }
    };
    let mut options = serde_json::Map::new();
    for key in [
        "locale",
        "cover_badge",
        "date_format",
        "date_format_short",
        "site_subtitle",
        "site_tagline",
        "site_description",
        "feed_time_style",
        "feed_time_custom",
        "feed_hide_time",
        "comment_moderation",
        "comment_scope",
        "giscus_repo",
        "giscus_repo_id",
        "giscus_category_id",
        "comment_provider",
        "comment_embed_url",
    ] {
        options.insert(key.into(), json!(settings.get(key)));
    }
    for key in [
        "post_max_chars",
        "post_max_images",
        "upload_max_mb",
        "article_max_bytes",
        "thumb_size",
        "thumb_quality",
        "avatar_size",
        "reading_chars_per_minute",
        "comment_rate_seconds",
    ] {
        options.insert(key.into(), json!(settings.number(key)));
    }
    let mut state = json!({"admin":false,"setup":false,"profile":{"name":profile("name","site_name"),"bio":profile("bio","profile_bio"),"handle":profile("handle","profile_handle"),"avatar":null,"cover":null},"options":options,"plugins":crate::plugins::public(app).await?,"capabilities":{"protocol":1,"read_only":false}});
    state["profile"]["avatar"] =
        json!(crate::media::profile_url(app, crate::media::ProfileImage::Avatar).await?);
    state["profile"]["cover"] =
        json!(crate::media::profile_url(app, crate::media::ProfileImage::Cover).await?);
    Ok(state)
}

async fn state(app: &App, session: &Session) -> Result<Value> {
    let settings = Settings::load(app).await?;
    let mut state = public_state(app, &settings).await?;
    let setup: bool = sqlx::query_scalar("SELECT NOT EXISTS(SELECT 1 FROM users)")
        .fetch_one(&app.db)
        .await?;
    state["setup"] = json!(setup);
    state["admin"] = json!(session.user.is_some());
    state["csrf"] = json!(session.csrf);
    if session.user.is_some() {
        state["version"] = json!(env!("CARGO_PKG_VERSION"));
        state["netlify"] = json!({"site":settings.get("netlify_site"),"has_token":!settings.get("netlify_token").is_empty(),"auto_deploy":settings.enabled("auto_deploy"),"auto_deploy_debounce":settings.number("auto_deploy_debounce")});
        state["comment_settings"] = json!({"smtp_host":settings.get("smtp_host"),"smtp_port":settings.number("smtp_port"),"smtp_user":settings.get("smtp_user"),"notify_email":settings.get("notify_email"),"akismet_key":settings.get("akismet_key")});
    }
    Ok(state)
}

pub(crate) async fn posts_filtered(
    app: &App,
    params: &HashMap<String, String>,
    source: Option<(&str, Option<String>)>,
) -> Result<Value> {
    let limit = params
        .get("limit")
        .map(|s| s.parse::<i64>().unwrap_or(0))
        .unwrap_or(30)
        .clamp(1, 100);
    let search: String = text(params, "q").trim().chars().take(200).collect();
    let pattern = format!("%{search}%");
    let hidden = if source.is_none() {
        crate::plugins::hidden_sources(app).await?
    } else {
        Vec::new()
    };
    let hidden = json!(hidden).to_string();
    let (source, prefix) = source.unwrap_or(("", None));
    let prefix = prefix.unwrap_or_default();
    let filter = "content LIKE ? AND source NOT IN (SELECT value FROM json_each(?)) AND (?='' OR source=?) AND (?='' OR source_key LIKE ?)";
    let total: i64 = sqlx::query_scalar(&format!("SELECT COUNT(*) FROM posts WHERE {filter}"))
        .bind(&pattern)
        .bind(&hidden)
        .bind(source)
        .bind(source)
        .bind(&prefix)
        .bind(&prefix)
        .fetch_one(&app.db)
        .await?;
    let c = cursor(params, true)?;
    let extra = if c.is_some() {
        " AND (pinned,created_at,id) < (?,?,?)"
    } else {
        ""
    };
    let sql = format!(
        "SELECT p.*, (SELECT COUNT(*) FROM post_likes l WHERE l.post_id=p.id) AS like_count, (SELECT COUNT(*) FROM comments c WHERE c.kind='post' AND c.target_id=p.id AND c.status='approved') AS comment_count FROM posts p WHERE {filter}{extra} ORDER BY pinned DESC,created_at DESC,id DESC LIMIT ?"
    );
    let mut query = sqlx::query(&sql)
        .bind(&pattern)
        .bind(&hidden)
        .bind(source)
        .bind(source)
        .bind(&prefix)
        .bind(&prefix);
    if let Some(v) = &c {
        query = query
            .bind(v[0].as_i64().unwrap())
            .bind(v[1].as_str().unwrap())
            .bind(v[2].as_i64().unwrap());
    }
    let mut items: Vec<Value> = query
        .bind(limit + 1)
        .fetch_all(&app.db)
        .await?
        .into_iter()
        .map(row_json)
        .collect();
    let more = items.len() > limit as usize;
    items.truncate(limit as usize);
    let next = next_cursor(&items, more, true);
    for item in &mut items {
        let refs: Vec<String> =
            serde_json::from_str(item["images"].as_str().unwrap_or("[]")).unwrap_or_default();
        let mut images = Vec::new();
        for reference in refs {
            if let Some(value) = upload_info(app, &reference).await {
                images.push(value);
            } else if let Some(value) = crate::plugins::resolve_ref(app, &reference).await? {
                images.push(value);
            }
        }
        item["images"] = json!(images);
        item["meta"] =
            serde_json::from_str(item["meta"].as_str().unwrap_or("")).unwrap_or(json!([]));
        if let Some(reference) = item["meta"]
            .get("avatar")
            .and_then(Value::as_str)
            .map(str::to_owned)
        {
            item["meta"]["avatar"] = upload_info(app, &reference).await.unwrap_or(Value::Null);
        }
    }
    Ok(json!({"posts":items,"total":total,"next_cursor":next}))
}

pub fn upload_name(name: &str) -> bool {
    let Some((stem, ext)) = name.rsplit_once('.') else {
        return false;
    };
    stem.len() == 32
        && stem
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
        && matches!(ext, "jpg" | "png" | "webp" | "gif" | "avif")
}
async fn upload_info(app: &App, reference: &str) -> Option<Value> {
    let name = reference.strip_prefix("up:")?;
    if !upload_name(name) {
        return None;
    }
    let root = tokio::fs::canonicalize(app.data.join("uploads"))
        .await
        .ok()?;
    let path = tokio::fs::canonicalize(root.join(name)).await.ok()?;
    if !path.starts_with(&root) {
        return None;
    }
    let bytes = tokio::fs::metadata(&path).await.ok()?.len();
    let (width, height) = if name.ends_with(".avif") {
        let (width, height, _) = crate::media::inspect(app, &path, &app.shutdown)
            .await
            .ok()?;
        (width, height)
    } else {
        tokio::task::spawn_blocking(move || image::image_dimensions(path))
            .await
            .ok()?
            .ok()?
    };
    Some(
        json!({"ref":reference,"url":format!("/image?upload={name}"),"thumb":format!("/image?upload={name}&thumb=1"),"width":width,"height":height,"bytes":bytes}),
    )
}

pub async fn image_file(
    State(app): State<App>,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Response> {
    for (key, kind) in [
        ("avatar", crate::media::ProfileImage::Avatar),
        ("cover", crate::media::ProfileImage::Cover),
    ] {
        if params.contains_key(key) {
            return crate::media::profile_response(&app, kind).await;
        }
    }
    if !params.contains_key("upload")
        && let Some(hash) = params.get("gravatar")
    {
        return crate::media::gravatar(&app, hash).await;
    }
    if !params.contains_key("upload")
        && let Some(response) = crate::plugins::image(&app, &params).await?
    {
        return Ok(response);
    }
    let name = text(&params, "upload");
    if !upload_name(name) {
        return Err(Error::Http(StatusCode::NOT_FOUND, "图片不存在".into()));
    }
    let missing = || Error::Http(StatusCode::NOT_FOUND, "图片不存在".into());
    let root = tokio::fs::canonicalize(app.data.join("uploads"))
        .await
        .map_err(|_| missing())?;
    let path = tokio::fs::canonicalize(root.join(name))
        .await
        .map_err(|_| missing())?;
    if !path.starts_with(root) {
        return Err(missing());
    }
    let thumbnail = if text(&params, "thumb") == "1" {
        match crate::plugins::thumbnail(&app, &format!("up:{name}")).await {
            Some(path) => Some(path),
            None => match crate::media::cached_thumbnail(&app, name, &path).await {
                Ok(path) => Some(path),
                Err(error) => {
                    tracing::warn!(error=%error,"缩略图生成失败，返回原图");
                    None
                }
            },
        }
    } else {
        None
    };
    let mime = if thumbnail.is_some() {
        "image/webp"
    } else {
        match name.rsplit('.').next().unwrap() {
            "jpg" => "image/jpeg",
            "png" => "image/png",
            "gif" => "image/gif",
            "avif" => "image/avif",
            _ => "image/webp",
        }
    };
    let path = thumbnail.unwrap_or(path);
    let mut service = tower_http::services::ServeFile::new_with_mime(path, &mime.parse().unwrap());
    let response = service
        .try_call(axum::http::Request::new(axum::body::Body::empty()))
        .await
        .map_err(|_| missing())?;
    Ok(response.into_response())
}

async fn articles(app: &App, params: &HashMap<String, String>, admin: bool) -> Result<Value> {
    if params.contains_key("id") {
        let id = text(params, "id").parse::<i64>().unwrap_or(0);
        let row = sqlx::query("SELECT a.*, (SELECT COUNT(*) FROM comments c WHERE c.kind='article' AND c.target_id=a.id AND c.status='approved') AS comment_count FROM articles a WHERE a.id=? AND (draft=0 OR ?)").bind(id).bind(admin).fetch_optional(&app.db).await?;
        let mut article = row
            .map(row_json)
            .ok_or_else(|| Error::Http(StatusCode::NOT_FOUND, "文章不存在".into()))?;
        article["html"] = json!(crate::render::article(&article));
        return Ok(json!({"article":article}));
    }
    let draft = i64::from(!matches!(text(params, "draft"), "" | "0"));
    if draft != 0 && !admin {
        return Err(Error::Http(
            StatusCode::UNAUTHORIZED,
            "请先登录管理后台".into(),
        ));
    }
    let limit = params
        .get("limit")
        .map(|s| s.parse::<i64>().unwrap_or(0))
        .unwrap_or(30)
        .clamp(1, 100);
    let search: String = text(params, "q").trim().chars().take(200).collect();
    let tag: String = text(params, "tag").chars().take(20).collect();
    let pattern = format!("%{search}%");
    let filter = "draft=? AND (title LIKE ? OR content LIKE ? OR tags LIKE ?) AND (?='' OR instr(',' || tags || ',',?)>0)";
    let tag_match = format!(",{tag},");
    let total: i64 = sqlx::query_scalar(&format!("SELECT COUNT(*) FROM articles WHERE {filter}"))
        .bind(draft)
        .bind(&pattern)
        .bind(&pattern)
        .bind(&pattern)
        .bind(&tag)
        .bind(&tag_match)
        .fetch_one(&app.db)
        .await?;
    let c = cursor(params, false)?;
    let extra = if c.is_some() {
        " AND (created_at,id)<(?,?)"
    } else {
        ""
    };
    let sql = format!(
        "SELECT a.*, (SELECT COUNT(*) FROM comments c WHERE c.kind='article' AND c.target_id=a.id AND c.status='approved') AS comment_count FROM articles a WHERE {filter}{extra} ORDER BY created_at DESC,id DESC LIMIT ?"
    );
    let mut query = sqlx::query(&sql)
        .bind(draft)
        .bind(&pattern)
        .bind(&pattern)
        .bind(&pattern)
        .bind(&tag)
        .bind(&tag_match);
    if let Some(v) = &c {
        query = query
            .bind(v[0].as_str().unwrap())
            .bind(v[1].as_i64().unwrap());
    }
    let mut items: Vec<Value> = query
        .bind(limit + 1)
        .fetch_all(&app.db)
        .await?
        .into_iter()
        .map(row_json)
        .collect();
    let more = items.len() > limit as usize;
    items.truncate(limit as usize);
    let next = next_cursor(&items, more, false);
    for item in &mut items {
        crate::render::summarize(item);
    }
    let mut result = json!({"articles":items,"total":total,"next_cursor":next});
    if draft == 0 && text(params, "cursor").is_empty() {
        let rows: Vec<(String, i64)> = sqlx::query_as(
            "SELECT tags,COUNT(*) FROM articles WHERE draft=0 AND tags<>'' GROUP BY tags",
        )
        .fetch_all(&app.db)
        .await?;
        let mut tags = serde_json::Map::new();
        for (names, count) in rows {
            for name in names.split(',') {
                let n = tags.get(name).and_then(Value::as_i64).unwrap_or(0);
                tags.insert(name.into(), json!(n + count));
            }
        }
        result["tags"] = if tags.is_empty() {
            json!([])
        } else {
            Value::Object(tags)
        };
    }
    Ok(result)
}
