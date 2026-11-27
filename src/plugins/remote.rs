use super::{ExportSnapshot, Plugin, import, process};
use crate::{api::App, config::Settings, tasks};
use anyhow::{Context, Result, bail};
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, HashMap},
    path::{Path, PathBuf},
};
use tokio_util::sync::CancellationToken;

pub(super) struct Remote {
    pub manifest: Value,
    pub directory: PathBuf,
    pub installation: Value,
}

impl Remote {
    fn supports(&self, method: &str) -> bool {
        self.manifest["methods"]
            .as_array()
            .unwrap()
            .iter()
            .any(|value| value == method)
    }

    async fn invoke(
        &self,
        app: &App,
        method: &str,
        params: Value,
        ctx: Option<&tasks::Context>,
    ) -> Result<Value> {
        if !self.supports(method) {
            bail!("插件没有声明 {method} 能力");
        }
        let settings = Settings::load(app).await?;
        let state = settings
            .get(&format!("plugin:{}:state", self.id()))
            .to_owned();
        let input = json!({
            "protocol": 1, "plugin":self.id(), "capabilities":self.manifest["capabilities"], "method":method, "params":params,
            "config":super::config(self, &settings).0,
            "state":if state.is_empty() { Value::Null } else { serde_json::from_str::<Value>(&state)? },
        });
        process::request(
            &self.directory.join("plugin"),
            &input,
            ctx.map(|ctx| &ctx.stop).unwrap_or(&app.shutdown),
            ctx,
            Some(app),
        )
        .await
    }

    async fn save_state(&self, app: &App, response: &Value) -> Result<()> {
        if let Some(state) = response.get("state") {
            sqlx::query("INSERT INTO settings(key,value) VALUES(?,?) ON CONFLICT(key) DO UPDATE SET value=excluded.value")
                .bind(format!("plugin:{}:state", self.id())).bind(state.to_string()).execute(&app.db).await?;
        }
        Ok(())
    }

    pub async fn check(
        &self,
        stop: &CancellationToken,
        ctx: Option<&tasks::Context>,
    ) -> Result<()> {
        let response = process::request(
            &self.directory.join("plugin"),
            &json!({"protocol":1,"method":"check","params":{},"config":{},"state":null}),
            stop,
            ctx,
            None,
        )
        .await?;
        let result = &response["result"];
        if result["protocol"] != 1
            || result["id"] != self.manifest["id"]
            || result["version"] != self.manifest["version"]
        {
            bail!("插件自检返回的协议、标识或版本与 plugin.json 不一致");
        }
        Ok(())
    }
}

#[derive(Deserialize)]
struct Post {
    key: String,
    content: String,
    #[serde(default)]
    created_at: String,
    #[serde(default)]
    link: String,
    #[serde(default)]
    meta: Value,
    #[serde(default)]
    images: Vec<String>,
}

#[async_trait]
impl Plugin for Remote {
    fn id(&self) -> &str {
        self.manifest["id"].as_str().unwrap()
    }
    fn manifest(&self) -> &Value {
        &self.manifest
    }
    fn installation(&self) -> Option<&Value> {
        Some(&self.installation)
    }
    fn frontend_directory(&self, _root: &Path) -> PathBuf {
        self.directory.join("frontend")
    }
    fn interval(&self, config: &Settings) -> i64 {
        if self.supports("sync") {
            config.number("sync_minutes") * 60
        } else {
            0
        }
    }

    async fn sync(&self, app: &App, ctx: &tasks::Context, params: Value) -> Result<Value> {
        let response = self.invoke(app, "sync", params, Some(ctx)).await?;
        let posts: Vec<Post> =
            serde_json::from_value(response.get("posts").cloned().unwrap_or(json!([])))
                .context("插件动态格式无效")?;
        if posts.len() > 1000 {
            bail!("插件单次返回动态超过 1000 条");
        }
        let mut added = 0;
        for post in posts {
            ctx.check()?;
            if post.key.is_empty() || post.key.len() > 500 || post.images.len() > 16 {
                bail!("插件动态标识或图片数量无效");
            }
            if !post.link.is_empty() {
                crate::network::parse_url(&post.link)?;
            }
            let created_at = if post.created_at.is_empty() {
                app.config.now()
            } else {
                chrono::DateTime::parse_from_rfc3339(&post.created_at)
                    .context("插件动态时间必须为 RFC3339")?;
                post.created_at
            };
            if import::known(app, self.id(), &post.key).await? {
                continue;
            }
            let mut images = Vec::new();
            for url in post.images {
                images.push(import::fetch_image(app, ctx, &url, 0, "").await?);
            }
            added += import::save(
                app,
                ctx,
                self.id(),
                import::Post {
                    key: post.key,
                    content: post.content,
                    created_at,
                    link: post.link,
                    meta: post.meta,
                    images,
                },
            )
            .await?;
        }
        let _content = app.content_lock.write().await;
        ctx.check()?;
        self.save_state(app, &response).await?;
        Ok(json!({"added":added,"plugin_result":response["result"]}))
    }

    async fn data(
        &self,
        app: &App,
        params: &HashMap<String, String>,
        admin: bool,
    ) -> crate::error::Result<Value> {
        Ok(self
            .invoke(app, "data", json!({"query":params,"admin":admin}), None)
            .await
            .map_err(tasks::api_error)?["result"]
            .clone())
    }

    async fn admin(&self, app: &App, input: Value) -> crate::error::Result<Value> {
        let _operation = super::mutation_guard(app, self.id())?;
        let response = self
            .invoke(app, "admin", input, None)
            .await
            .map_err(tasks::api_error)?;
        let _content = app.content_lock.write().await;
        self.save_state(app, &response)
            .await
            .map_err(tasks::api_error)?;
        Ok(response["result"].clone())
    }

    async fn export_snapshot(&self, app: &App) -> crate::error::Result<Option<ExportSnapshot>> {
        if !self.supports("export") {
            return Ok(None);
        }
        let response = self
            .invoke(app, "export", json!({}), None)
            .await
            .map_err(tasks::api_error)?;
        Ok(Some(ExportSnapshot {
            data: response["result"].clone(),
            references: vec![],
        }))
    }

    fn export_data(
        &self,
        data: &Value,
        _posts: &[Value],
        _media: &BTreeMap<String, Value>,
    ) -> Value {
        data.clone()
    }
    fn export_page(&self, data: &Value) -> Option<String> {
        data["html"].as_str().map(ammonia::clean)
    }
    fn render_post(&self, post: &Value) -> Option<String> {
        post["meta"]["html"].as_str().map(ammonia::clean)
    }
}
