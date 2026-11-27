use super::{process, remote::Remote};
use crate::{api::App, tasks};
use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use std::{
    path::{Component, Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use tokio::process::Command;

fn identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value.as_bytes()[0].is_ascii_alphanumeric()
        && value
            .bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, b'-' | b'_'))
}

fn relative(value: &str) -> bool {
    !value.is_empty() && !value.contains('\\') && Path::new(value).components().all(|part| matches!(part, Component::Normal(name) if !name.to_string_lossy().starts_with('.')))
}

pub(super) fn manifest(value: &Value) -> Result<()> {
    if value["protocol"] != 1 {
        bail!("插件协议版本不受支持，需要 protocol: 1");
    }
    for key in ["id", "binary"] {
        if !value[key].as_str().is_some_and(identifier) {
            bail!("插件 {key} 必须为 1–64 位小写字母、数字、连字符或下划线");
        }
    }
    if !value["name"]
        .as_str()
        .is_some_and(|name| !name.is_empty() && name.len() <= 200)
    {
        bail!("插件名称无效");
    }
    if !value["version"].as_str().is_some_and(|version| {
        regex::Regex::new(r"^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$")
            .unwrap()
            .is_match(version)
    }) {
        bail!("插件版本必须为 major.minor.patch");
    }
    let methods = value["methods"]
        .as_array()
        .context("插件必须声明 methods 数组")?;
    if methods
        .iter()
        .any(|method| !matches!(method.as_str(), Some("sync" | "data" | "admin" | "export")))
    {
        bail!("插件声明了不受支持的方法");
    }
    if value.get("setting_keys").is_some() {
        bail!("独立插件不能映射主程序配置键");
    }
    if let Some(capabilities) = value.get("capabilities")
        && !capabilities
            .as_array()
            .is_some_and(|items| items.iter().all(|item| item == "llm.generate"))
    {
        bail!("插件主程序能力声明无效");
    }
    let fields = value["fields"]
        .as_array()
        .context("插件必须声明 fields 数组")?;
    let mut names = std::collections::HashSet::new();
    for field in fields {
        let key = field["key"].as_str().context("插件配置项缺少 key")?;
        if !identifier(key)
            || matches!(
                key,
                "enabled" | "removed" | "state" | "last_error" | "last_sync"
            )
            || !names.insert(key)
        {
            bail!("插件配置键无效或重复：{key}");
        }
        if !field["label"].is_string() {
            bail!("插件配置项缺少 label");
        }
        if let Some(pattern) = field["pattern"].as_str() {
            regex::Regex::new(pattern).context("插件配置正则表达式无效")?;
        }
        match field["type"].as_str() {
            Some("number")
                if field["min"].is_i64()
                    && field["max"].is_i64()
                    && field["min"].as_i64() <= field["max"].as_i64() => {}
            Some("select") if field["options"].is_array() || field["options"].is_object() => {}
            Some("text") | None => {}
            _ => bail!("插件配置项类型或约束无效：{key}"),
        }
    }
    if let Some(frontend) = value.get("frontend") {
        for key in ["module", "style"] {
            if let Some(file) = frontend.get(key)
                && !file.as_str().is_some_and(relative)
            {
                bail!("插件前端路径必须位于 frontend 目录内");
            }
        }
    }
    if let Some(page) = value.get("page") {
        let route = page["route"].as_str().context("插件页面缺少 route")?;
        if !route.starts_with('/')
            || !relative(&route[1..])
            || route.contains(['?', '#', '%'])
            || !page["title"].is_string()
        {
            bail!("插件页面路由或标题无效");
        }
        if matches!(
            route.split('/').nth(1),
            Some(
                "api"
                    | "admin"
                    | "articles"
                    | "article"
                    | "assets"
                    | "plugins"
                    | "image"
                    | "health"
                    | "embed"
            )
        ) {
            bail!("插件页面路由与主程序冲突");
        }
        if !methods.iter().any(|method| method == "export") {
            bail!("独立页面插件必须实现 export，保证静态站可用");
        }
    }
    Ok(())
}

pub fn source(kind: &str, input: &str) -> Result<String> {
    match kind {
        "github" => {
            let url = url::Url::parse(input).context("GitHub 仓库地址无效")?;
            let path = url.path().trim_end_matches('/').trim_end_matches(".git");
            let parts: Vec<_> = path.trim_start_matches('/').split('/').collect();
            if url.scheme() != "https"
                || url.host_str() != Some("github.com")
                || !url.username().is_empty()
                || url.password().is_some()
                || url.port().is_some()
                || url.query().is_some()
                || url.fragment().is_some()
                || parts.len() != 2
                || parts.iter().any(|part| {
                    part.is_empty()
                        || part.starts_with('.')
                        || !part
                            .bytes()
                            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-' | b'.'))
                })
            {
                bail!("请输入公开 GitHub 仓库地址：https://github.com/作者/仓库");
            }
            Ok(format!("https://github.com/{}/{}.git", parts[0], parts[1]))
        }
        "local" => {
            let path = Path::new(input);
            if !path.is_absolute() {
                bail!("本地仓库必须使用服务机器上的绝对路径");
            }
            let path = path.canonicalize().context("本地仓库不存在或不可读取")?;
            if !path.is_dir() || !path.join(".git").exists() {
                bail!("所选目录不是 Git 仓库");
            }
            Ok(path.to_str().context("仓库路径必须为 UTF-8")?.into())
        }
        _ => bail!("请选择 GitHub 仓库或本地仓库"),
    }
}

pub async fn environment() -> Value {
    let mut result = serde_json::Map::new();
    for name in ["git", "cargo", "rustc", "cc"] {
        let output = tokio::time::timeout(
            Duration::from_secs(5),
            Command::new(name)
                .arg("--version")
                .kill_on_drop(true)
                .output(),
        )
        .await;
        let version = output
            .ok()
            .and_then(Result::ok)
            .filter(|output| output.status.success())
            .map(|output| {
                String::from_utf8_lossy(&output.stdout)
                    .lines()
                    .next()
                    .unwrap_or("")
                    .to_owned()
            });
        result.insert(
            name.into(),
            json!({"available":version.is_some(),"version":version}),
        );
    }
    Value::Object(result)
}

pub async fn load(app: &App) -> Result<()> {
    sqlx::query("CREATE TABLE IF NOT EXISTS plugin_installations(id TEXT PRIMARY KEY, manifest TEXT NOT NULL, installation TEXT NOT NULL)").execute(&app.db).await?;
    let rows: Vec<(String, String, String)> =
        sqlx::query_as("SELECT id,manifest,installation FROM plugin_installations")
            .fetch_all(&app.db)
            .await?;
    for (id, description, installation) in rows {
        let description: Value = serde_json::from_str(&description)?;
        manifest(&description)?;
        let installation: Value = serde_json::from_str(&installation)?;
        let generation = installation["generation"]
            .as_str()
            .filter(|name| identifier(name))
            .context("插件安装记录无效")?;
        let directory = app.data.join("extensions").join(&id).join(generation);
        if app
            .plugins
            .get(&id)
            .is_ok_and(|plugin| plugin.installation().is_none())
        {
            bail!("插件标识重复：{id}");
        }
        app.plugins.insert(Arc::new(Remote {
            manifest: description,
            directory,
            installation,
        }));
    }
    Ok(())
}

fn copy_tree(
    source: &Path,
    destination: &Path,
    stop: &tokio_util::sync::CancellationToken,
) -> Result<()> {
    let mut pending = vec![(source.to_owned(), destination.to_owned())];
    while let Some((source, destination)) = pending.pop() {
        if stop.is_cancelled() {
            bail!("任务已取消");
        }
        std::fs::create_dir_all(&destination)?;
        for entry in std::fs::read_dir(source)? {
            let entry = entry?;
            if entry.file_name() == ".git" || entry.file_name() == "target" {
                continue;
            }
            let target = destination.join(entry.file_name());
            let kind = entry.file_type()?;
            if kind.is_dir() {
                pending.push((entry.path(), target));
            } else if kind.is_file() {
                std::fs::copy(entry.path(), target)?;
            } else {
                bail!("插件仓库不能包含符号链接或特殊文件");
            }
        }
    }
    Ok(())
}

async fn checkout(ctx: &tasks::Context, kind: &str, input: &str) -> Result<PathBuf> {
    let source = source(kind, input)?;
    ctx.progress("准备插件源码", 0, Some(4)).await?;
    let repository = ctx.directory.join("source");
    if kind == "github" {
        let mut command = Command::new("git");
        command
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .args([
                "-c",
                "credential.helper=",
                "-c",
                "protocol.file.allow=never",
                "clone",
                "--depth",
                "1",
                "--single-branch",
                "--no-tags",
                "--",
                &source,
            ])
            .arg(&repository);
        process::build(&mut command, ctx).await?;
    } else {
        let source = PathBuf::from(&source);
        let repository = repository.clone();
        let stop = ctx.stop.clone();
        tokio::task::spawn_blocking(move || copy_tree(&source, &repository, &stop)).await??;
    }
    Ok(repository)
}

pub fn subdirectory(value: &str) -> Result<String> {
    if value.is_empty() {
        return Ok(String::new());
    }
    if !relative(value) || value.len() > 300 {
        bail!("插件子目录必须是仓库内的相对路径");
    }
    Ok(value.into())
}

pub async fn discover(ctx: &tasks::Context, kind: &str, input: &str) -> Result<Value> {
    let repository = checkout(ctx, kind, input).await?;
    let root = repository.clone();
    let stop = ctx.stop.clone();
    let plugins = tokio::task::spawn_blocking(move || -> Result<Vec<Value>> {
        let mut pending = vec![(root.clone(),0)];
        let mut found = Vec::new();
        while let Some((directory,depth)) = pending.pop() {
            if stop.is_cancelled() { bail!("任务已取消"); }
            let manifest_file = directory.join("plugin.json");
            if manifest_file.is_file() {
                let description: Value = serde_json::from_slice(&std::fs::read(manifest_file)?)?;
                manifest(&description)?;
                let path = directory.strip_prefix(&root)?.to_str().context("插件路径必须为 UTF-8")?;
                subdirectory(path)?;
                found.push(json!({"id":description["id"],"name":description["name"],"version":description["version"],"path":path}));
            }
            if depth >= 4 { continue; }
            for entry in std::fs::read_dir(&directory)? {
                let entry = entry?;
                if entry.file_name().to_string_lossy().starts_with('.') || entry.file_name() == "target" { continue; }
                if entry.file_type()?.is_dir() { pending.push((entry.path(), depth+1)); }
            }
        }
        found.sort_by(|a,b| a["path"].as_str().cmp(&b["path"].as_str()));
        if found.is_empty() { bail!("仓库内没有找到 plugin.json（最多扫描四层子目录）"); }
        Ok(found)
    }).await??;
    Ok(json!({"plugins":plugins}))
}

pub async fn run(
    app: &App,
    ctx: &tasks::Context,
    kind: &str,
    input: &str,
    path: &str,
) -> Result<Value> {
    let source = source(kind, input)?;
    let path = subdirectory(path)?;
    let root = checkout(ctx, kind, input).await?;
    let repository = root
        .join(&path)
        .canonicalize()
        .context("插件子目录不存在")?;
    if !repository.starts_with(root.canonicalize()?) {
        bail!("插件路径不能离开仓库");
    }
    let description: Value = serde_json::from_slice(
        &tokio::fs::read(repository.join("plugin.json"))
            .await
            .context("所选目录缺少 plugin.json")?,
    )?;
    manifest(&description)?;
    let id = description["id"].as_str().unwrap();
    let previous = app.plugins.get(id).ok();
    if previous
        .as_ref()
        .is_some_and(|plugin| plugin.installation().is_none())
    {
        bail!("不能覆盖内置插件：{id}");
    }
    let _operation = match &previous {
        Some(_) => Some(app.plugins.mutation_guard(id)?),
        None => None,
    };
    if let Some(route) = description["page"]["route"].as_str()
        && app.plugins.all().iter().any(|plugin| {
            plugin.id() != id
                && plugin.manifest()["page"]["route"]
                    .as_str()
                    .is_some_and(|existing| {
                        existing.trim_end_matches('/') == route.trim_end_matches('/')
                    })
        })
    {
        bail!("插件页面路由已被占用");
    }
    if !repository.join("Cargo.toml").is_file() || !repository.join("Cargo.lock").is_file() {
        bail!("插件仓库必须包含独立的 Cargo.toml 和 Cargo.lock");
    }
    ctx.progress("编译插件", 1, Some(4)).await?;
    let target = ctx.directory.join("target");
    let temporary = ctx.directory.join("tmp");
    tokio::fs::create_dir(&temporary).await?;
    let mut command = Command::new("cargo");
    command
        .current_dir(&repository)
        .env("TMPDIR", &temporary)
        .env(
            "CARGO_HOME",
            std::env::var_os("CARGO_HOME")
                .unwrap_or_else(|| app.data.join("runtime/plugin-cargo").into_os_string()),
        )
        .args([
            "build",
            "--release",
            "--locked",
            "--bin",
            description["binary"].as_str().unwrap(),
            "--target-dir",
        ])
        .arg(&target);
    process::build(&mut command, ctx).await?;
    let stage = ctx.directory.join("package");
    tokio::fs::create_dir(&stage).await?;
    tokio::fs::copy(
        target
            .join("release")
            .join(description["binary"].as_str().unwrap()),
        stage.join("plugin"),
    )
    .await?;
    if let Some(frontend) = description.get("frontend") {
        let src = repository.join("frontend");
        for field in ["module", "style"] {
            if let Some(file) = frontend[field].as_str()
                && !src.join(file).is_file()
            {
                bail!("插件缺少声明的前端文件：{file}");
            }
        }
        let dest = stage.join("frontend");
        let stop = ctx.stop.clone();
        tokio::task::spawn_blocking(move || copy_tree(&src, &dest, &stop)).await??;
    }
    let generation = crate::auth::random_token()?;
    let installation = json!({"kind":kind,"source":source,"path":path,"generation":generation});
    let mut plugin = Remote {
        manifest: description.clone(),
        directory: stage.clone(),
        installation,
    };
    ctx.progress("验证插件协议", 2, Some(4)).await?;
    plugin.check(&ctx.stop, Some(ctx)).await?;
    ctx.progress("安装插件", 3, Some(4)).await?;
    let _content = tokio::select! { _=ctx.stop.cancelled()=>bail!("任务已取消"), guard=app.content_lock.write()=>guard };
    ctx.check()?;
    let parent = app.data.join("extensions").join(id);
    tokio::fs::create_dir_all(&parent).await?;
    let destination = parent.join(&generation);
    // Work directories can be on another filesystem; stage on the final filesystem before activation.
    let pending = parent.join(format!(".install-{generation}"));
    let stop = ctx.stop.clone();
    let pending_copy = pending.clone();
    let copy = tokio::task::spawn_blocking(move || copy_tree(&stage, &pending_copy, &stop)).await?;
    if let Err(error) = copy {
        let _ = tokio::fs::remove_dir_all(&pending).await;
        return Err(error);
    }
    if ctx.stop.is_cancelled() {
        tokio::fs::remove_dir_all(&pending).await?;
        bail!("任务已取消");
    }
    tokio::fs::rename(&pending, &destination).await?;
    plugin.directory = destination.clone();
    let saved = async {
        let mut tx = app.db.begin_with("BEGIN IMMEDIATE").await?;
        sqlx::query("INSERT INTO plugin_installations(id,manifest,installation) VALUES(?,?,?) ON CONFLICT(id) DO UPDATE SET manifest=excluded.manifest,installation=excluded.installation")
            .bind(id).bind(description.to_string()).bind(plugin.installation.to_string()).execute(&mut *tx).await?;
        sqlx::query("DELETE FROM settings WHERE key=?").bind(format!("plugin:{id}:removed")).execute(&mut *tx).await?;
        tx.commit().await
    }.await;
    if let Err(error) = saved {
        let _ = tokio::fs::remove_dir_all(&destination).await;
        return Err(error.into());
    }
    app.plugins.insert(Arc::new(plugin));
    Ok(json!({"id":id,"version":description["version"],"installed":true}))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_unsafe_sources_and_manifest_boundaries() {
        for url in [
            "http://github.com/a/b",
            "https://user:secret@github.com/a/b",
            "https://github.com/a/b?ref=x",
            "https://github.com/a/b/tree/main",
            "file:///tmp/repo",
            "https://github.com/a/b%20--upload-pack=x",
        ] {
            assert!(source("github", url).is_err(), "{url}");
        }
        assert_eq!(
            source("github", "https://github.com/owner/plugin.git/").unwrap(),
            "https://github.com/owner/plugin.git"
        );
        let template: Value =
            serde_json::from_str(include_str!("../../examples/standalone-plugin/plugin.json"))
                .unwrap();
        manifest(&template).unwrap();
        for (path, value) in [
            ("/id", json!("../core")),
            ("/protocol", json!(2)),
            ("/frontend/module", json!("../private")),
            ("/page/route", json!("/admin")),
            ("/fields/0/key", json!("_enabled")),
            ("/fields/1/min", json!("not-a-number")),
        ] {
            let mut invalid = template.clone();
            *invalid.pointer_mut(path).unwrap() = value;
            assert!(manifest(&invalid).is_err(), "{path}");
        }
        let mut invalid = template;
        invalid["setting_keys"] = json!({"enabled":"admin_password"});
        assert!(manifest(&invalid).is_err());
    }
}
