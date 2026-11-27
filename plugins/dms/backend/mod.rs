use super::Plugin;
use crate::{api::App, config::Settings, tasks};
use anyhow::{Result, bail};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{collections::BTreeMap, path::Path, sync::LazyLock};
use tokio_util::sync::CancellationToken;

pub struct Dms;
static MANIFEST: LazyLock<Value> =
    LazyLock::new(|| serde_json::from_str(include_str!("manifest.json")).unwrap());
#[derive(Clone, Deserialize, Serialize)]
struct DesktopPlugin {
    name: String,
    #[serde(default)]
    version: String,
}
#[derive(Clone, PartialEq, Deserialize, Serialize)]
struct Package {
    package: String,
    version: String,
}
#[derive(Deserialize, Serialize)]
struct Snapshot {
    directory: String,
    plugins: BTreeMap<String, DesktopPlugin>,
    kernel: String,
    dms: Option<Package>,
    #[serde(default)]
    revision: u64,
}
async fn read_snapshot(
    directory: &Path,
    packages: &Path,
    kernel: String,
    stop: &CancellationToken,
) -> Result<Snapshot> {
    let mut entries = tokio::fs::read_dir(directory.join("plugins"))
        .await
        .map_err(|_| anyhow::anyhow!("DMS 插件目录不存在或不可读，请检查路径与权限"))?;
    let mut plugins = BTreeMap::new();
    while let Some(entry) = entries.next_entry().await? {
        if stop.is_cancelled() {
            bail!("任务已取消");
        }
        let path = entry.path().join("plugin.json");
        let bytes = match tokio::fs::read(path).await {
            Ok(bytes) => bytes,
            Err(error)
                if error.kind() == std::io::ErrorKind::NotFound
                    || error.kind() == std::io::ErrorKind::NotADirectory =>
            {
                continue;
            }
            Err(_) => bail!("DMS 插件清单不可读"),
        };
        let value: Value =
            serde_json::from_slice(&bytes).map_err(|_| anyhow::anyhow!("DMS 插件清单格式无效"))?;
        let id = value["id"]
            .as_str()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| anyhow::anyhow!("DMS 插件清单缺少 ID"))?;
        let plugin: DesktopPlugin = serde_json::from_value(value.clone())
            .map_err(|_| anyhow::anyhow!("DMS 插件清单格式无效"))?;
        if plugin.name.is_empty() {
            bail!("DMS 插件清单缺少名称");
        }
        plugins.insert(id.to_owned(), plugin);
    }
    let mut dms = None;
    let mut package_paths = Vec::new();
    match tokio::fs::read_dir(packages).await {
        Ok(mut entries) => {
            while let Some(entry) = entries.next_entry().await? {
                let name = entry.file_name();
                let name = name.to_string_lossy();
                if ["dux-shell", "dank-material-shell", "dms"]
                    .iter()
                    .any(|p| name.starts_with(p))
                {
                    package_paths.push(entry.path().join("desc"));
                }
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    package_paths.sort();
    for path in package_paths {
        if stop.is_cancelled() {
            bail!("任务已取消");
        }
        let Ok(desc) = tokio::fs::read_to_string(path).await else {
            continue;
        };
        let lines: Vec<_> = desc.lines().collect();
        let field = |name| {
            lines
                .windows(2)
                .find(|pair| pair[0] == name)
                .map(|pair| pair[1])
        };
        if let (Some(name), Some(version)) = (field("%NAME%"), field("%VERSION%")) {
            let base = name
                .strip_suffix("-git")
                .or_else(|| name.strip_suffix("-bin"))
                .unwrap_or(name);
            if ["dux-shell", "dank-material-shell", "dms"].contains(&base) {
                dms = Some(Package {
                    package: name.into(),
                    version: version.into(),
                });
                break;
            }
        }
    }
    Ok(Snapshot {
        directory: tokio::fs::canonicalize(directory)
            .await?
            .to_string_lossy()
            .into_owned(),
        plugins,
        kernel,
        dms,
        revision: 0,
    })
}
fn changes(old: &Snapshot, current: &Snapshot) -> BTreeMap<&'static str, String> {
    if old.directory != current.directory {
        return BTreeMap::new();
    }
    let mut installed = Vec::new();
    let mut updated = Vec::new();
    let mut changes = BTreeMap::new();
    for (id, plugin) in &current.plugins {
        match old.plugins.get(id) {
            None => installed.push(plugin.name.clone()),
            Some(previous)
                if !previous.version.is_empty()
                    && !plugin.version.is_empty()
                    && previous.version != plugin.version =>
            {
                updated.push(format!(
                    "{}（{} → {}）",
                    plugin.name, previous.version, plugin.version
                ))
            }
            _ => {}
        }
    }
    if !installed.is_empty() {
        changes.insert(
            "installed",
            format!("安装了新的 DMS 插件：{}", installed.join("、")),
        );
    }
    if !updated.is_empty() {
        changes.insert(
            "updated",
            format!("DMS 插件版本更新：{}", updated.join("、")),
        );
    }
    if old.kernel != current.kernel {
        changes.insert(
            "kernel",
            format!("Linux 运行内核已切换：{} → {}", old.kernel, current.kernel),
        );
    }
    if let (Some(old), Some(current)) = (&old.dms, &current.dms)
        && old != current
    {
        changes.insert(
            "dms",
            format!("DMS 软件包版本更新：{} → {}", old.version, current.version),
        );
    }
    changes
}
fn event_key(directory: &str, revision: u64, kind: &str, content: &str) -> String {
    let json = json!([directory, revision, kind, content]).to_string();
    let mut legacy = String::new();
    for ch in json.chars() {
        if ch == '/' {
            legacy.push_str("\\/");
        } else if ch.is_ascii() {
            legacy.push(ch);
        } else {
            for unit in ch.encode_utf16(&mut [0; 2]) {
                legacy.push_str(&format!("\\u{unit:04x}"));
            }
        }
    }
    format!("snapshot:{}", crate::auth::digest(&legacy))
}
#[async_trait]
impl Plugin for Dms {
    fn id(&self) -> &'static str {
        "dms"
    }
    fn manifest(&self) -> &'static Value {
        &MANIFEST
    }
    async fn migrate(&self, app: &App) -> Result<()> {
        sqlx::query("DELETE FROM settings WHERE key IN ('plugin:dms:events_db','plugin:dms:cursor','plugin:dms:music_day')").execute(&app.db).await?;
        Ok(())
    }
    async fn sync(&self, app: &App, ctx: &tasks::Context, _params: Value) -> Result<Value> {
        let config = super::config(self, &Settings::load(app).await?);
        ctx.progress("读取桌面快照", 0, None).await?;
        let kernel = tokio::fs::read_to_string("/proc/sys/kernel/osrelease")
            .await?
            .trim()
            .to_owned();
        let mut current = read_snapshot(
            Path::new(config.get("config_dir")),
            Path::new("/var/lib/pacman/local"),
            kernel,
            &ctx.stop,
        )
        .await?;
        let _content = app.content_lock.write().await;
        let mut tx = app.db.begin_with("BEGIN IMMEDIATE").await?;
        ctx.check()?;
        let previous: Option<String> =
            sqlx::query_scalar("SELECT value FROM settings WHERE key='plugin:dms:snapshot'")
                .fetch_optional(&mut *tx)
                .await?;
        let old = previous
            .as_deref()
            .map(serde_json::from_str::<Snapshot>)
            .transpose()?;
        let revision = old.as_ref().map(|s| s.revision).unwrap_or(0);
        let changes = old
            .as_ref()
            .map(|old| changes(old, &current))
            .unwrap_or_default();
        current.revision = revision + 1;
        let now = app.config.now();
        let mut added = 0;
        for (kind, content) in &changes {
            ctx.check()?;
            let key = event_key(&current.directory, revision, kind, content);
            added+=sqlx::query("INSERT INTO posts(content,created_at,updated_at,source,source_key) SELECT ?,?,?,'dms',? WHERE NOT EXISTS(SELECT 1 FROM posts WHERE source='dms' AND source_key=?) AND NOT EXISTS(SELECT 1 FROM plugin_tombstones WHERE plugin='dms' AND key=?)")
                .bind(content).bind(&now).bind(&now).bind(&key).bind(&key).bind(&key).execute(&mut *tx).await?.rows_affected();
        }
        ctx.check()?;
        sqlx::query("INSERT INTO settings(key,value) VALUES('plugin:dms:snapshot',?) ON CONFLICT(key) DO UPDATE SET value=excluded.value").bind(serde_json::to_string(&current)?).execute(&mut *tx).await?;
        tx.commit().await?;
        ctx.progress(
            "桌面快照已保存",
            changes.len() as i64,
            Some(changes.len() as i64),
        )
        .await?;
        Ok(json!({"added":added}))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn event_keys_preserve_unicode_and_path_encoding() {
        assert_eq!(
            event_key(
                "/home/test/桌面",
                4,
                "updated",
                "插件版本更新：音乐（1 → 2）"
            ),
            "snapshot:655adf01a3dc7b7e8f63569b92d960624a56e2775eaf251e454e19e3f527efdc"
        );
    }
    #[tokio::test]
    async fn snapshot_checks_manifests_and_compares_all_events() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join("plugins/a")).unwrap();
        std::fs::write(
            root.path().join("plugins/a/plugin.json"),
            r#"{"id":"a","name":"Example","version":"1"}"#,
        )
        .unwrap();
        let stop = CancellationToken::new();
        let mut old = read_snapshot(
            root.path(),
            &root.path().join("pacman"),
            "6.0".into(),
            &stop,
        )
        .await
        .unwrap();
        let mut current = read_snapshot(
            root.path(),
            &root.path().join("pacman"),
            "6.1".into(),
            &stop,
        )
        .await
        .unwrap();
        current.plugins.get_mut("a").unwrap().version = "2".into();
        current.plugins.insert(
            "b".into(),
            DesktopPlugin {
                name: "New".into(),
                version: "1".into(),
            },
        );
        old.dms = Some(Package {
            package: "dms".into(),
            version: "1".into(),
        });
        current.dms = Some(Package {
            package: "dms".into(),
            version: "2".into(),
        });
        assert_eq!(changes(&old, &current).len(), 4);
        current.directory = "/different".into();
        assert!(changes(&old, &current).is_empty());
        std::fs::write(root.path().join("plugins/a/plugin.json"), "{}").unwrap();
        assert!(
            read_snapshot(
                root.path(),
                &root.path().join("pacman"),
                "6.1".into(),
                &stop
            )
            .await
            .is_err()
        );
    }
}
