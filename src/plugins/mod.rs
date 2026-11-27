#[path = "../../plugins/dms/backend/mod.rs"]
mod dms;
#[path = "../../plugins/douban/backend/mod.rs"]
mod douban;
mod feed;
mod import;
#[path = "../../plugins/rss/backend/mod.rs"]
mod rss;
#[path = "../../plugins/wallpapers/backend/mod.rs"]
mod wallpapers;
#[path = "../../plugins/weibo/backend/mod.rs"]
mod weibo;

use crate::{
    api::App,
    config::Settings,
    error::{Error, Result},
    tasks,
};
use async_trait::async_trait;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, HashMap},
    path::PathBuf,
    sync::{Arc, LazyLock},
};

pub struct ExportSnapshot {
    pub data: Value,
    pub references: Vec<String>,
}

#[async_trait]
pub trait Plugin: Sync + Send {
    fn manifest(&self) -> &'static Value;
    fn id(&self) -> &'static str;
    async fn migrate(&self, _app: &App) -> anyhow::Result<()> {
        Ok(())
    }
    async fn data(
        &self,
        _app: &App,
        _params: &HashMap<String, String>,
        _admin: bool,
    ) -> Result<Value> {
        Err(Error::not_found("插件没有独立数据接口"))
    }
    async fn admin(&self, _app: &App, _input: Value) -> Result<Value> {
        Err(Error::invalid("插件不支持此管理操作"))
    }
    async fn upload(
        &self,
        _app: &App,
        _input: Value,
        _files: Vec<crate::media::Upload>,
    ) -> Result<Value> {
        Err(Error::invalid("插件不支持上传"))
    }
    async fn sync(&self, app: &App, ctx: &tasks::Context, params: Value) -> anyhow::Result<Value>;
    async fn config_changed(
        &self,
        _tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
        _config: &Settings,
    ) -> Result<()> {
        Ok(())
    }
    async fn resolve_ref(&self, _app: &App, _reference: &str) -> Result<Option<Value>> {
        Ok(None)
    }
    async fn image(
        &self,
        _app: &App,
        _params: &HashMap<String, String>,
    ) -> Result<Option<axum::response::Response>> {
        Ok(None)
    }
    fn interval(&self, config: &Settings) -> i64 {
        config.number("sync_minutes") * 60
    }
    async fn references(&self, _app: &App, _reference: &str) -> Result<bool> {
        Ok(false)
    }
    async fn delete_media(&self, _app: &App, _reference: &str) -> Result<()> {
        Ok(())
    }
    async fn export_snapshot(&self, _app: &App) -> Result<Option<ExportSnapshot>> {
        Ok(None)
    }
    /// 完整备份时随库打包的资源目录（键 → 目录），恢复时写回设置表。
    async fn backup_paths(&self, _app: &App) -> Result<Vec<(String, PathBuf)>> {
        Ok(Vec::new())
    }
    fn export_data(
        &self,
        _data: &Value,
        _posts: &[Value],
        _media: &BTreeMap<String, Value>,
    ) -> Value {
        Value::Null
    }
    fn export_page(&self, _data: &Value) -> Option<String> {
        None
    }
    fn render_post(&self, _post: &Value) -> Option<String> {
        None
    }
    async fn export_source(&self, _app: &App, _reference: &str) -> Result<Option<PathBuf>> {
        Ok(None)
    }
    async fn thumbnail(&self, _app: &App, _reference: &str) -> Option<PathBuf> {
        None
    }
    fn post_refs(&self, _meta: &Value) -> Vec<String> {
        Vec::new()
    }
    async fn posts_purged(
        &self,
        _tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    ) -> Result<Vec<String>> {
        Ok(Vec::new())
    }
}

static PLUGINS: [&'static dyn Plugin; 5] = [
    &rss::Rss,
    &dms::Dms,
    &weibo::Weibo,
    &douban::Douban,
    &wallpapers::Wallpapers,
];
static GATES: LazyLock<HashMap<&'static str, Arc<tokio::sync::Mutex<()>>>> = LazyLock::new(|| {
    PLUGINS
        .iter()
        .map(|plugin| (plugin.id(), Arc::new(tokio::sync::Mutex::new(()))))
        .collect()
});

pub fn mutation_guard(id: &str) -> Result<tokio::sync::OwnedMutexGuard<()>> {
    get(id)?;
    GATES[id].clone().try_lock_owned().map_err(|_| {
        Error::Http(
            axum::http::StatusCode::CONFLICT,
            "插件正在执行任务，请等待完成或先取消任务".into(),
        )
    })
}

pub fn frontend_directories(root: &std::path::Path) -> Vec<(&'static str, PathBuf)> {
    PLUGINS
        .iter()
        .filter(|plugin| plugin.manifest().get("frontend").is_some())
        .map(|plugin| (plugin.id(), root.join(plugin.id()).join("frontend")))
        .collect()
}

pub fn assets(mut router: axum::Router<App>, root: &std::path::Path) -> axum::Router<App> {
    for (id, directory) in frontend_directories(root) {
        router = router.nest_service(
            &format!("/plugins/{id}"),
            tower_http::services::ServeDir::new(directory),
        );
    }
    router
}

pub fn get(id: &str) -> Result<&'static dyn Plugin> {
    PLUGINS
        .iter()
        .copied()
        .find(|p| p.id() == id)
        .ok_or_else(|| Error::not_found("插件不存在"))
}
pub async fn migrate(app: &App) -> anyhow::Result<()> {
    for plugin in PLUGINS {
        plugin.migrate(app).await?;
    }
    import::recover(app).await?;
    Ok(())
}
fn setting_key(plugin: &dyn Plugin, key: &str) -> String {
    plugin.manifest()["setting_keys"][key]
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| format!("plugin:{}:{key}", plugin.id()))
}
pub fn config(plugin: &dyn Plugin, settings: &Settings) -> Settings {
    let mut values = std::collections::BTreeMap::new();
    values.insert(
        "_enabled".into(),
        settings.get(&setting_key(plugin, "enabled")).into(),
    );
    for field in plugin.manifest()["fields"]
        .as_array()
        .expect("plugin fields")
    {
        let key = field["key"].as_str().unwrap();
        let value = settings.get(&setting_key(plugin, key));
        values.insert(
            key.into(),
            if value.is_empty() {
                field["default"]
                    .as_str()
                    .map(str::to_owned)
                    .unwrap_or_else(|| {
                        field
                            .get("default")
                            .map(Value::to_string)
                            .unwrap_or_default()
                    })
            } else {
                value.into()
            },
        );
    }
    Settings(values)
}
pub async fn public(app: &App) -> Result<Value> {
    let settings = Settings::load(app).await?;
    let mut values = serde_json::Map::new();
    for plugin in PLUGINS {
        let manifest = plugin.manifest();
        let enabled = config(plugin, &settings).enabled("_enabled");
        let mut item = json!({"name":manifest["name"],"color":manifest["label_color"]});
        if manifest["post_images_full"] == true {
            item["full_images"] = json!(true);
        }
        for asset in ["module", "style"] {
            if let Some(file) = manifest["frontend"][asset].as_str() {
                item[asset] = json!(format!("/plugins/{}/{file}", plugin.id()));
            }
        }
        if enabled
            && manifest.get("page").is_some()
            && manifest["page_setting"]
                .as_str()
                .is_none_or(|key| !config(plugin, &settings).get(key).is_empty())
        {
            item["page"] = manifest["page"].clone();
        }
        values.insert(plugin.id().into(), item);
    }
    Ok(Value::Object(values))
}
pub async fn hidden_sources(app: &App) -> Result<Vec<String>> {
    let settings = Settings::load(app).await?;
    Ok(PLUGINS
        .iter()
        .filter_map(|plugin| {
            let config = config(*plugin, &settings);
            plugin.manifest()["timeline_field"]
                .as_str()
                .filter(|field| config.enabled("_enabled") && !config.enabled(field))
                .map(|_| plugin.id().to_owned())
        })
        .collect())
}
pub async fn admin_list(app: &App) -> Result<Value> {
    let settings = Settings::load(app).await?;
    let mut result = Vec::new();
    for plugin in PLUGINS {
        let config = config(plugin, &settings);
        let mut item = plugin.manifest().clone();
        for field in item["fields"].as_array_mut().unwrap() {
            field["value"] = json!(config.get(field["key"].as_str().unwrap()));
        }
        let prefix = format!("plugin:{}:", plugin.id());
        item["enabled"] = json!(config.enabled("_enabled"));
        item["post_count"] = json!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM posts WHERE source=?")
                .bind(plugin.id())
                .fetch_one(&app.db)
                .await?
        );
        item["last_sync"] = json!(settings.number(&format!("{prefix}last_sync")));
        item["last_error"] = json!(settings.get(&format!("{prefix}last_error")));
        item["removed"] = json!(settings.enabled(&format!("{prefix}removed")));
        result.push(item);
    }
    Ok(json!({"plugins":result}))
}
pub async fn save(app: &App, input: &Value) -> Result<Value> {
    let plugin = get(input["id"].as_str().unwrap_or(""))?;
    let _operation = mutation_guard(plugin.id())?;
    let mut changes = Vec::new();
    let mut values = input["values"].as_object().cloned().unwrap_or_default();
    if input["enabled"] == 1 || input["enabled"] == true {
        for (key, value) in config(plugin, &Settings::load(app).await?).0 {
            values.entry(key).or_insert(json!(value));
        }
    }
    for field in plugin.manifest()["fields"].as_array().unwrap() {
        let key = field["key"].as_str().unwrap();
        let Some(value) = values.get(key) else {
            continue;
        };
        let raw = match value {
            Value::String(s) => s.trim().to_owned(),
            Value::Number(n) => n.to_string(),
            _ => return Err(Error::invalid("插件设置格式无效")),
        };
        if field["required"] == true && raw.is_empty() {
            return Err(Error::invalid(format!(
                "{} 不能为空",
                field["label"].as_str().unwrap_or(key)
            )));
        }
        if !raw.is_empty()
            && let Some(pattern) = field["pattern"].as_str()
            && !regex::Regex::new(pattern)
                .expect("shipped plugin pattern")
                .is_match(&raw)
        {
            return Err(Error::invalid(format!(
                "{} 格式无效",
                field["label"].as_str().unwrap_or(key)
            )));
        }
        match field["type"].as_str().unwrap_or("text") {
            "number" => {
                let number = raw
                    .parse::<i64>()
                    .map_err(|_| Error::invalid("插件设置必须为整数"))?;
                if number < field["min"].as_i64().unwrap()
                    || number > field["max"].as_i64().unwrap()
                {
                    return Err(Error::invalid("插件设置超出范围"));
                }
            }
            "select" => {
                let options = &field["options"];
                if !(options.get(&raw).is_some()
                    || options
                        .as_array()
                        .is_some_and(|a| raw.parse::<usize>().is_ok_and(|n| n < a.len())))
                {
                    return Err(Error::invalid("插件选项无效"));
                }
            }
            _ => {
                if raw.chars().count() > field["max"].as_u64().unwrap_or(200) as usize {
                    return Err(Error::invalid("插件设置过长"));
                }
            }
        }
        changes.push((setting_key(plugin, key), raw));
    }
    if let Some(enabled) = input.get("enabled") {
        changes.push((
            setting_key(plugin, "enabled"),
            if enabled == 1 || enabled == true {
                "1"
            } else {
                "0"
            }
            .into(),
        ));
    }
    let _guard = app.content_lock.write().await;
    let mut settings = Settings::load(app).await?;
    settings.0.extend(changes.iter().cloned());
    let mut tx = app.db.begin_with("BEGIN IMMEDIATE").await?;
    for (key, value) in changes {
        sqlx::query("INSERT INTO settings(key,value) VALUES(?,?) ON CONFLICT(key) DO UPDATE SET value=excluded.value").bind(key).bind(value).execute(&mut *tx).await?;
    }
    sqlx::query("DELETE FROM settings WHERE key=?")
        .bind(format!("plugin:{}:removed", plugin.id()))
        .execute(&mut *tx)
        .await?;
    plugin
        .config_changed(&mut tx, &config(plugin, &settings))
        .await?;
    tx.commit().await?;
    Ok(json!({"ok":true}))
}
pub async fn enabled(app: &App, id: &str) -> Result<&'static dyn Plugin> {
    let plugin = get(id)?;
    if !config(plugin, &Settings::load(app).await?).enabled("_enabled") {
        return Err(Error::invalid("插件未启用"));
    }
    Ok(plugin)
}
pub async fn enqueue(app: &App, id: &str, params: Value) -> Result<Value> {
    enabled(app, id).await?;
    let task = tasks::Task::PluginSync {
        plugin: id.into(),
        params,
    };
    let id = tasks::submit(&app.tasks, &task)
        .await
        .map_err(tasks::api_error)?;
    Ok(json!({"ok":true,"task_id":id}))
}
pub async fn run(
    app: &App,
    ctx: &tasks::Context,
    id: &str,
    params: Value,
) -> anyhow::Result<Value> {
    get(id)?;
    let _operation = tokio::select! {_=ctx.stop.cancelled()=>anyhow::bail!("任务已取消"), guard=GATES[id].clone().lock_owned()=>guard};
    let plugin = enabled(app, id).await?;
    let result = plugin.sync(app, ctx, params).await;
    let _guard = app.content_lock.write().await;
    let mut tx = app.db.begin_with("BEGIN IMMEDIATE").await?;
    for (key, value) in [
        ("last_sync", chrono::Utc::now().timestamp().to_string()),
        (
            "last_error",
            result
                .as_ref()
                .err()
                .map(|e| e.to_string())
                .unwrap_or_default(),
        ),
    ] {
        sqlx::query("INSERT INTO settings(key,value) VALUES(?,?) ON CONFLICT(key) DO UPDATE SET value=excluded.value").bind(format!("plugin:{id}:{key}")).bind(value).execute(&mut *tx).await?;
    }
    tx.commit().await?;
    result
}
pub async fn schedules(app: &App) -> Result<Vec<tasks::Schedule>> {
    let settings = Settings::load(app).await?;
    Ok(PLUGINS
        .iter()
        .filter_map(|plugin| {
            let config = config(*plugin, &settings);
            (config.enabled("_enabled") && plugin.interval(&config) > 0).then(|| tasks::Schedule {
                key: format!("plugin:{}", plugin.id()),
                task: tasks::Task::PluginSync {
                    plugin: plugin.id().into(),
                    params: json!({"all":false}),
                },
                interval: plugin.interval(&config),
            })
        })
        .collect())
}
pub async fn resolve_ref(app: &App, reference: &str) -> Result<Option<Value>> {
    for plugin in PLUGINS {
        if let Some(value) = plugin.resolve_ref(app, reference).await? {
            return Ok(Some(value));
        }
    }
    Ok(None)
}
pub async fn image(
    app: &App,
    params: &HashMap<String, String>,
) -> Result<Option<axum::response::Response>> {
    for plugin in PLUGINS {
        if let Some(response) = plugin.image(app, params).await? {
            return Ok(Some(response));
        }
    }
    Ok(None)
}
pub async fn export_snapshots(app: &App) -> Result<BTreeMap<String, ExportSnapshot>> {
    let settings = Settings::load(app).await?;
    let mut snapshots = BTreeMap::new();
    for plugin in PLUGINS {
        if config(plugin, &settings).enabled("_enabled")
            && let Some(snapshot) = plugin.export_snapshot(app).await?
        {
            snapshots.insert(plugin.id().into(), snapshot);
        }
    }
    Ok(snapshots)
}

/// 所有插件声明的备份资源目录（插件 ID，设置键，目录）。
pub async fn backup_paths(app: &App) -> Result<Vec<(&'static str, String, PathBuf)>> {
    let mut paths = Vec::new();
    for plugin in PLUGINS {
        for (key, directory) in plugin.backup_paths(app).await? {
            paths.push((plugin.id(), key, directory));
        }
    }
    Ok(paths)
}

pub fn export_data(
    snapshots: &BTreeMap<String, ExportSnapshot>,
    posts: &[Value],
    media: &BTreeMap<String, Value>,
) -> (Value, Value) {
    let mut items = serde_json::Map::new();
    let mut metadata = serde_json::Map::new();
    for plugin in PLUGINS {
        if let Some(snapshot) = snapshots.get(plugin.id()) {
            let data = plugin.export_data(&snapshot.data, posts, media);
            items.insert(plugin.id().into(), data["items"].clone());
            metadata.insert(plugin.id().into(), data["meta"].clone());
        }
    }
    (Value::Object(items), Value::Object(metadata))
}

pub async fn export_source(app: &App, reference: &str) -> Result<Option<PathBuf>> {
    for plugin in PLUGINS {
        if let Some(path) = plugin.export_source(app, reference).await? {
            return Ok(Some(path));
        }
    }
    Ok(None)
}
pub async fn references(app: &App, reference: &str) -> Result<bool> {
    for plugin in PLUGINS {
        if plugin.references(app, reference).await? {
            return Ok(true);
        }
    }
    Ok(false)
}
pub async fn delete_media(app: &App, reference: &str) -> Result<()> {
    for plugin in PLUGINS {
        plugin.delete_media(app, reference).await?;
    }
    Ok(())
}
pub async fn thumbnail(app: &App, reference: &str) -> Option<PathBuf> {
    for plugin in PLUGINS {
        if let Some(path) = plugin.thumbnail(app, reference).await {
            return Some(path);
        }
    }
    None
}

pub fn post_refs(source: &str, meta: &str) -> Vec<String> {
    match (get(source), serde_json::from_str::<Value>(meta)) {
        (Ok(plugin), Ok(meta)) => plugin.post_refs(&meta),
        _ => Vec::new(),
    }
}

pub async fn remove(app: &App, input: &Value, uninstall: bool) -> Result<Value> {
    let plugin = get(input["id"].as_str().unwrap_or(""))?;
    let id = plugin.id();
    let _operation = mutation_guard(id)?;
    let _content = app.content_lock.write().await;
    let purge = !uninstall || input["purge"] == 1 || input["purge"] == true;
    let mut tx = app.db.begin_with("BEGIN IMMEDIATE").await?;
    let mut refs = Vec::new();
    let mut deleted = 0;
    if purge {
        let rows: Vec<(String, String)> =
            sqlx::query_as("SELECT images,meta FROM posts WHERE source=?")
                .bind(id)
                .fetch_all(&mut *tx)
                .await?;
        deleted = rows.len();
        for (images, meta) in rows {
            refs.extend(serde_json::from_str::<Vec<String>>(&images).unwrap_or_default());
            refs.extend(post_refs(id, &meta));
        }
        sqlx::query("DELETE FROM comments WHERE kind='post' AND target_id IN (SELECT id FROM posts WHERE source=?)").bind(id).execute(&mut *tx).await?;
        sqlx::query(
            "DELETE FROM post_likes WHERE post_id IN (SELECT id FROM posts WHERE source=?)",
        )
        .bind(id)
        .execute(&mut *tx)
        .await?;
        sqlx::query("DELETE FROM posts WHERE source=?")
            .bind(id)
            .execute(&mut *tx)
            .await?;
        refs.extend(plugin.posts_purged(&mut tx).await?);
    }
    sqlx::query("DELETE FROM plugin_tombstones WHERE plugin=?")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    if uninstall {
        let prefix = format!("plugin:{id}:");
        sqlx::query("DELETE FROM settings WHERE substr(key,1,?)=?")
            .bind(prefix.len() as i64)
            .bind(&prefix)
            .execute(&mut *tx)
            .await?;
        for (key, value) in [("enabled", "0"), ("removed", "1")] {
            sqlx::query("INSERT INTO settings(key,value) VALUES(?,?) ON CONFLICT(key) DO UPDATE SET value=excluded.value")
                .bind(setting_key(plugin,key))
                .bind(value)
                .execute(&mut *tx)
                .await?;
        }
    }
    tx.commit().await?;
    refs.sort();
    refs.dedup();
    let mut files = 0;
    for reference in refs {
        let path = reference
            .strip_prefix("up:")
            .filter(|name| crate::api::upload_name(name))
            .map(|name| app.data.join("uploads").join(name));
        let existed = path.as_ref().is_some_and(|p| p.is_file());
        crate::media::delete_unused(app, &reference).await?;
        if existed && path.is_some_and(|p| !p.exists()) {
            files += 1;
        }
    }
    Ok(json!({"ok":true,"purged":purge,"deleted":deleted,"deleted_files":files}))
}
