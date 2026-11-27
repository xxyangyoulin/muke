use crate::{
    api::App,
    error::{Error, Result},
};
use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier, password_hash::SaltString};
use axum::{
    extract::{ConnectInfo, Request, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    middleware::Next,
    response::{IntoResponse, Response},
};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::net::{IpAddr, SocketAddr};
use subtle::ConstantTimeEq;

#[derive(Clone, Default)]
pub struct Session {
    pub id: String,
    pub csrf: String,
    pub user: Option<i64>,
}
impl Session {
    pub fn require_admin(&self) -> Result<i64> {
        self.user.ok_or_else(Error::unauthorized)
    }
}
pub fn random_token() -> Result<String> {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).map_err(|e| Error::Internal(e.to_string()))?;
    Ok(hex::encode(bytes))
}
pub fn digest(value: &str) -> String {
    hex::encode(Sha256::digest(value.as_bytes()))
}
fn cookie(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(header::COOKIE)?
        .to_str()
        .ok()?
        .split(';')
        .find_map(|s| s.trim().strip_prefix("muke_session="))
}
fn set_cookie(token: &str, secure: bool, max_age: i64) -> HeaderValue {
    HeaderValue::from_str(&format!(
        "muke_session={token}; Path=/; HttpOnly; SameSite=Lax; Max-Age={max_age}{}",
        if secure { "; Secure" } else { "" }
    ))
    .expect("random token header")
}
async fn create(
    app: &App,
    user: Option<i64>,
    csrf: Option<&str>,
) -> Result<(Session, HeaderValue)> {
    let token = random_token()?;
    let id = digest(&token);
    let csrf = csrf
        .map(str::to_owned)
        .map(Ok)
        .unwrap_or_else(random_token)?;
    let now = chrono::Utc::now().timestamp();
    sqlx::query("DELETE FROM sessions WHERE expires_at<?")
        .bind(now)
        .execute(&app.db)
        .await?;
    sqlx::query("INSERT INTO sessions(id,csrf,user_id,expires_at) VALUES(?,?,?,?)")
        .bind(&id)
        .bind(&csrf)
        .bind(user)
        .bind(now + 86400)
        .execute(&app.db)
        .await?;
    Ok((
        Session { id, csrf, user },
        set_cookie(&token, app.config.cookie_secure, 86400),
    ))
}
pub async fn middleware(State(app): State<App>, mut request: Request, next: Next) -> Response {
    match load(&app, request.headers(), request.method(), request.uri()).await {
        Ok((session, header)) => {
            if request.method() != axum::http::Method::GET {
                let supplied = request
                    .headers()
                    .get("X-CSRF-Token")
                    .and_then(|s| s.to_str().ok())
                    .unwrap_or("");
                if session.csrf.is_empty()
                    || !bool::from(session.csrf.as_bytes().ct_eq(supplied.as_bytes()))
                {
                    return Error::Http(StatusCode::FORBIDDEN, "页面已过期，请刷新后重试".into())
                        .into_response();
                }
            }
            request.extensions_mut().insert(session);
            let mut response = next.run(request).await;
            if let Some(header) = header {
                response.headers_mut().append(header::SET_COOKIE, header);
            }
            response
        }
        Err(error) => error.into_response(),
    }
}
async fn load(
    app: &App,
    headers: &HeaderMap,
    method: &axum::http::Method,
    uri: &axum::http::Uri,
) -> Result<(Session, Option<HeaderValue>)> {
    if let Some(token) =
        cookie(headers).filter(|s| s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit()))
    {
        let id = digest(token);
        let row: Option<(String, Option<i64>)> =
            sqlx::query_as("SELECT csrf,user_id FROM sessions WHERE id=? AND expires_at>?")
                .bind(&id)
                .bind(chrono::Utc::now().timestamp())
                .fetch_optional(&app.db)
                .await?;
        if let Some((csrf, user)) = row {
            return Ok((Session { id, csrf, user }, None));
        }
    }
    let action = uri
        .query()
        .and_then(|q| q.split('&').find_map(|p| p.strip_prefix("action=")))
        .unwrap_or("state");
    if method == axum::http::Method::GET && action == "state" {
        let (session, cookie) = create(app, None, None).await?;
        return Ok((session, Some(cookie)));
    }
    Ok((Session::default(), None))
}
#[derive(Deserialize)]
pub struct Credentials {
    pub username: String,
    pub password: String,
}
pub async fn authenticate(
    app: &App,
    session: &Session,
    peer: IpAddr,
    input: Credentials,
    setup: bool,
) -> Result<Response> {
    let name = input.username.trim().to_owned();
    let password = input.password;
    if password.len() > 200 {
        return Err(Error::invalid("密码过长"));
    }
    let identity = digest(&format!("{peer}|{name}"));
    if setup {
        if !app
            .config
            .setup_hosts
            .iter()
            .any(|host| host == "*" || host == &peer.to_string())
        {
            return Err(Error::Http(
                StatusCode::FORBIDDEN,
                "首次建号仅允许来自受信来源".into(),
            ));
        }
        if !(2..=40).contains(&name.chars().count()) || password.len() < 10 {
            return Err(Error::invalid("用户名需 2–40 字，密码需 10–200 位"));
        }
    } else {
        rate_limit(&app.db, "login", &identity, 5, 900).await?;
    }
    let permit = app
        .password_slots
        .clone()
        .acquire_owned()
        .await
        .map_err(|e| Error::Internal(e.to_string()))?;
    let id = if setup {
        let hash = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            hash_password(&password)
        })
        .await
        .map_err(|e| Error::Internal(e.to_string()))??;
        let result = sqlx::query("INSERT OR IGNORE INTO users(id,name,password) VALUES(1,?,?)")
            .bind(&name)
            .bind(hash)
            .execute(&app.db)
            .await?;
        if result.rows_affected() == 0 {
            return Err(Error::Http(
                StatusCode::CONFLICT,
                "管理员已设置，请登录".into(),
            ));
        }
        1
    } else {
        let row: Option<(i64, String)> =
            sqlx::query_as("SELECT id,password FROM users WHERE name=?")
                .bind(&name)
                .fetch_optional(&app.db)
                .await?;
        let hash = row.as_ref().map(|(_, hash)| hash.clone());
        let valid = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            hash.is_some_and(|hash| verify_password(&password, &hash))
        })
        .await
        .map_err(|e| Error::Internal(e.to_string()))?;
        if !valid {
            return Err(Error::Http(
                StatusCode::UNAUTHORIZED,
                "用户名或密码不正确".into(),
            ));
        }
        row.expect("verified user").0
    };
    sqlx::query("DELETE FROM rate_limits WHERE scope='login' AND identity=?")
        .bind(identity)
        .execute(&app.db)
        .await?;
    let (_, cookie) = create(app, Some(id), Some(&session.csrf)).await?;
    sqlx::query("DELETE FROM sessions WHERE id=?")
        .bind(&session.id)
        .execute(&app.db)
        .await?;
    Ok((
        [(header::SET_COOKIE, cookie)],
        axum::Json(serde_json::json!({"ok":true})),
    )
        .into_response())
}
pub async fn logout(app: &App, session: &Session) -> Result<Response> {
    sqlx::query("DELETE FROM sessions WHERE id=?")
        .bind(&session.id)
        .execute(&app.db)
        .await?;
    Ok((
        [(
            header::SET_COOKIE,
            set_cookie("", app.config.cookie_secure, 0),
        )],
        axum::Json(serde_json::json!({"ok":true})),
    )
        .into_response())
}
pub fn verify_password(password: &str, hash: &str) -> bool {
    if hash.starts_with("$2") {
        return bcrypt::verify(password, hash).unwrap_or(false);
    }
    PasswordHash::new(hash).is_ok_and(|hash| {
        Argon2::default()
            .verify_password(password.as_bytes(), &hash)
            .is_ok()
    })
}
pub fn hash_password(password: &str) -> Result<String> {
    let mut salt = [0u8; 16];
    getrandom::fill(&mut salt).map_err(|e| Error::Internal(e.to_string()))?;
    let salt = SaltString::encode_b64(&salt).map_err(|e| Error::Internal(e.to_string()))?;
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|h| h.to_string())
        .map_err(|e| Error::Internal(e.to_string()))
}
pub async fn rate_limit(
    db: &sqlx::SqlitePool,
    scope: &str,
    identity: &str,
    max: i64,
    window: i64,
) -> Result<()> {
    let now = chrono::Utc::now().timestamp();
    let attempts:i64=sqlx::query_scalar("INSERT INTO rate_limits(scope,identity,started_at,attempts) VALUES(?,?,?,1) ON CONFLICT(scope,identity) DO UPDATE SET attempts=CASE WHEN started_at<=? THEN 1 ELSE attempts+1 END, started_at=CASE WHEN started_at<=? THEN excluded.started_at ELSE started_at END RETURNING attempts")
        .bind(scope).bind(identity).bind(now).bind(now-window).bind(now-window).fetch_one(db).await?;
    if attempts > max {
        return Err(Error::Http(
            StatusCode::TOO_MANY_REQUESTS,
            "操作太频繁了，请稍后再试".into(),
        ));
    }
    Ok(())
}
pub fn peer(request: &Request) -> Result<IpAddr> {
    request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|p| p.0.ip())
        .ok_or_else(|| Error::Internal("请求缺少连接信息".into()))
}
